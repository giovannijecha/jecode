mod accounts;
mod models;
mod terminal;

pub use models::choose_model;
pub use terminal::Terminal;
#[cfg(test)]
pub(crate) use terminal::native_sign_in_fixture;

use crate::backend::{self, Services};
use crate::config::{Settings, Store};
use crate::redact::Redactor;

pub trait Prompter {
    fn message(&mut self, text: &str) -> Result<(), String>;
    fn input(&mut self, prompt: &str) -> Result<Option<String>, String>;
    fn secret(&mut self, prompt: &str) -> Result<Option<String>, String>;

    fn with_sign_in_cancel<T>(
        &mut self,
        operation: impl FnOnce(&mut Self, crate::cancel::Cancellation) -> Result<T, String>,
    ) -> Result<SignIn<T>, String>
    where
        Self: Sized,
    {
        operation(self, crate::cancel::Cancellation::default()).map(SignIn::Completed)
    }
}

pub enum SignIn<T> {
    Completed(T),
    Back,
}

pub fn configure(
    store: &Store,
    current: Option<&Settings>,
    ui: &mut impl Prompter,
) -> Result<Option<Settings>, String> {
    loop {
        let backend_id = choose_provider(ui)?;
        let Some(backend_id) = backend_id else {
            return cancelled(ui);
        };
        if backend_id == "openai" {
            match accounts::configure(store, current, ui)? {
                accounts::Outcome::Finished(settings) => return Ok(settings),
                accounts::Outcome::Back => continue,
            }
        }
        return configure_with(store, current, ui, backend_id, |id, key| {
            Services::for_backend(id, key.into())
        });
    }
}

fn choose_provider(ui: &mut impl Prompter) -> Result<Option<&'static str>, String> {
    let available = backend::available();
    loop {
        ui.message("Providers:")?;
        for (index, backend) in available.iter().enumerate() {
            let method = if backend.id == "openai" {
                "Continue with ChatGPT"
            } else {
                "API key"
            };
            ui.message(&format!("{}. {} · {method}", index + 1, backend.name))?;
        }
        let Some(answer) =
            answer(ui.input("Choose a provider number (/cancel leaves unchanged): ")?)
        else {
            return Ok(None);
        };
        if let Some(backend) = answer
            .parse::<usize>()
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| available.get(index))
        {
            return Ok(Some(backend.id));
        }
        ui.message("Choose one of the displayed backends.")?;
    }
}

fn configure_with(
    store: &Store,
    current: Option<&Settings>,
    ui: &mut impl Prompter,
    backend_id: &str,
    make_services: impl Fn(&str, &str) -> Result<Services, String>,
) -> Result<Option<Settings>, String> {
    let selected = backend::available()
        .iter()
        .find(|backend| backend.id == backend_id)
        .unwrap();
    let name = selected.name;
    let note = selected
        .key_url
        .map_or(String::new(), |url| format!("Get a key at {url}. "));
    ui.message(&format!("Jecode setup\nCredentials and model are saved in plain text in {}.\n{note}Type /cancel to leave setup.", store.path().display()))?;
    let (api_key, services) = loop {
        let saved_key = current
            .and_then(|settings| settings.credential_for(backend_id))
            .filter(|key| !key.is_empty());
        let prompt = if saved_key.is_some() {
            format!("{name} API key (hidden; Enter keeps the saved key): ")
        } else {
            format!("{name} API key (hidden): ")
        };
        let Some(key) = answer(ui.secret(&prompt)?) else {
            return cancelled(ui);
        };
        let key = if key.is_empty() {
            current
                .and_then(|settings| settings.credential_for(backend_id))
                .unwrap_or("")
                .into()
        } else {
            key
        };
        let services = match make_services(backend_id, &key) {
            Ok(services) => services,
            Err(error) => {
                ui.message(&Redactor::new(key).text(&error))?;
                continue;
            }
        };
        ui.message(&format!("Checking the key with {name}..."))?;
        match services.check_credential() {
            Ok(()) => break (key, services),
            Err(error) => ui.message(
                &Redactor::new(key.clone()).text(&format!("{error}\nTry another key, or /cancel.")),
            )?,
        }
    };
    let current_model = current
        .filter(|settings| settings.backend_id == backend_id && settings.has_model())
        .map(|settings| settings.model.as_str());
    let Some(model) = choose_model(&services, current_model, ui)? else {
        return cancelled(ui);
    };
    let mut settings = match current {
        Some(current) => current.clone(),
        None => Settings::selected(backend_id, api_key.clone(), model.clone())?,
    };
    settings.select_backend(backend_id, api_key.clone())?;
    settings.model = model;
    if let Some(current) = current {
        settings.auto_compact = current.auto_compact;
        if current.backend_id == settings.backend_id {
            settings.routing = current.routing.clone();
        }
        if current.backend_id == settings.backend_id && current.model == settings.model {
            settings.effort = current.effort;
        } else {
            settings.effort = crate::effort::Effort::Default;
        }
    }
    let redactor = Redactor::new(api_key);
    store
        .save(&settings)
        .map_err(|error| redactor.text(&error))?;
    let settings = store
        .load()
        .map_err(|error| redactor.text(&error))?
        .ok_or("Saved configuration could not be reopened")?;
    ui.message(&redactor.text(&format!("Saved. Model: {}", settings.model)))?;
    Ok(Some(settings))
}

pub(super) fn answer(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| text != "/cancel")
}

fn cancelled<T>(ui: &mut impl Prompter) -> Result<Option<T>, String> {
    ui.message("Setup cancelled; configuration was left unchanged.")?;
    Ok(None)
}

#[cfg(test)]
mod tests;
