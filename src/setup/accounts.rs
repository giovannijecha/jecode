use super::{Prompter, SignIn, answer, cancelled, choose_model};
use crate::{
    auth::{AccountStore, SignInUpdate},
    backend::Services,
    config::{Settings, Store},
};

pub(super) enum Outcome {
    Finished(Option<Settings>),
    Back,
}

pub(super) fn configure(
    config: &Store,
    current: Option<&Settings>,
    ui: &mut impl Prompter,
    provider: &str,
) -> Result<Outcome, String> {
    let (plan, new_account) = match provider {
        "openai" => ("ChatGPT", "ChatGPT account or workspace"),
        "anthropic" => ("Claude", "Claude account"),
        _ => return Err(format!("Unknown account provider: {provider}")),
    };
    let home = config.path().parent().expect("configuration directory");
    let accounts = AccountStore::new(home.to_path_buf(), provider);
    let saved = accounts.accounts()?;
    let existing = if saved.is_empty() {
        let Some(value) = answer(ui.input(&format!(
            "Continue with {plan} (Enter opens your browser; /cancel leaves unchanged): ",
        ))?) else {
            return cancelled(ui).map(Outcome::Finished);
        };
        if !value.is_empty() {
            ui.message(&format!("Press Enter to continue with {plan}."))?;
            return Ok(Outcome::Finished(None));
        }
        None
    } else {
        ui.message(&format!("{plan} accounts:"))?;
        for (index, account) in saved.iter().enumerate() {
            ui.message(&format!("{}. {}", index + 1, account.label))?;
        }
        ui.message(&format!("{}. Add a {new_account}", saved.len() + 1))?;
        loop {
            let Some(value) =
                answer(ui.input("Choose an account number (/cancel leaves unchanged): ")?)
            else {
                return cancelled(ui).map(Outcome::Finished);
            };
            if let Some(index) = value.parse::<usize>().ok().and_then(|n| n.checked_sub(1)) {
                if index == saved.len() {
                    break None;
                }
                if let Some(account) = saved.get(index) {
                    break Some(account);
                }
            }
            ui.message("Choose one of the displayed accounts.")?;
        }
    };
    let session = match existing.filter(|account| account.enabled) {
        Some(account) => accounts.session(&account.id)?,
        None => match ui.with_sign_in_cancel(|ui, cancellation| {
            accounts.sign_in(
                existing.map(|account| account.id.as_str()),
                cancellation,
                &mut |update| match update {
                    SignInUpdate::Browser(url) => {
                        ui.message(&format!("Continue with {plan} in your browser:\n{url}"))
                    }
                    SignInUpdate::Waiting => ui.message(&format!("Waiting for {plan} sign-in…")),
                },
            )
        })? {
            SignIn::Completed(session) => session,
            SignIn::Back => {
                ui.message(&format!(
                    "{plan} sign-in cancelled. Choose a provider to continue."
                ))?;
                return Ok(Outcome::Back);
            }
        },
    };
    let services = Services::for_credential(&session.credential())?;
    services.check_credential()?;
    let current_model = current
        .filter(|settings| settings.backend_id == provider)
        .map(|settings| settings.model.as_str());
    let Some(model) = choose_model(&services, current_model, ui)? else {
        return cancelled(ui).map(Outcome::Finished);
    };
    let mut settings = match current {
        Some(settings) => settings.clone(),
        None => Settings::selected(provider, String::new(), model.clone())?,
    };
    let same_model = settings.backend_id == provider && settings.model == model;
    settings.select_backend(provider, String::new())?;
    settings.model = model;
    if !same_model {
        settings.effort = crate::effort::Effort::Default;
    }
    config.save(&settings)?;
    accounts.activate(session.id())?;
    if provider == "openai" {
        ui.message("Using ChatGPT plan. Manage usage: https://chatgpt.com/settings/usage")?;
    } else {
        ui.message("Using Claude plan")?;
    }
    ui.message(&format!("Saved. Model: {}", settings.model))?;
    Ok(Outcome::Finished(Some(settings)))
}
