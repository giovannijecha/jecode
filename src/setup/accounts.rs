use super::{Prompter, answer, cancelled, choose_model};
use crate::{
    auth::{Credential, openai},
    backend::Services,
    cancel::Cancellation,
    config::{Settings, Store},
};

pub(super) fn configure(
    config: &Store,
    current: Option<&Settings>,
    ui: &mut impl Prompter,
) -> Result<Option<Settings>, String> {
    let home = config.path().parent().expect("configuration directory");
    let accounts = openai::Store::new(home.to_path_buf());
    let saved = accounts.accounts()?;
    let existing = if saved.is_empty() {
        let Some(value) = answer(ui.input(
            "Continue with ChatGPT (Enter opens your browser; /cancel leaves unchanged): ",
        )?) else {
            return cancelled(ui);
        };
        if !value.is_empty() {
            ui.message("Press Enter to continue with ChatGPT.")?;
            return Ok(None);
        }
        None
    } else {
        ui.message("ChatGPT accounts:")?;
        for (index, account) in saved.iter().enumerate() {
            ui.message(&format!("{}. {}", index + 1, account.label))?;
        }
        ui.message(&format!(
            "{}. Add a ChatGPT account or workspace",
            saved.len() + 1
        ))?;
        loop {
            let Some(value) =
                answer(ui.input("Choose an account number (/cancel leaves unchanged): ")?)
            else {
                return cancelled(ui);
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
        Some(account) => accounts.session(&account.client_id)?,
        None => accounts.sign_in(
            existing.map(|account| account.client_id.as_str()),
            Cancellation::default(),
            &mut |update| match update {
                openai::SignInUpdate::Browser(url) => {
                    ui.message(&format!("Continue with ChatGPT in your browser:\n{url}"))
                }
                openai::SignInUpdate::Waiting => ui.message("Waiting for ChatGPT sign-in…"),
            },
        )?,
    };
    let services = Services::for_credential(&Credential::openai(session.clone()))?;
    services.check_credential()?;
    let current_model = current
        .filter(|settings| settings.backend_id == "openai")
        .map(|settings| settings.model.as_str());
    let Some(model) = choose_model(&services, current_model, ui)? else {
        return cancelled(ui);
    };
    let mut settings = match current {
        Some(settings) => settings.clone(),
        None => Settings::selected("openai", String::new(), model.clone())?,
    };
    let same_model = settings.backend_id == "openai" && settings.model == model;
    settings.select_backend("openai", String::new())?;
    settings.model = model;
    if !same_model {
        settings.effort = crate::effort::Effort::Default;
    }
    config.save(&settings)?;
    accounts.activate(session.client_id())?;
    ui.message("Using ChatGPT plan. Manage usage: https://chatgpt.com/settings/usage")?;
    ui.message(&format!("Saved. Model: {}", settings.model))?;
    Ok(Some(settings))
}
