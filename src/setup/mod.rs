mod models;
mod terminal;

pub use models::choose_model;
pub use terminal::Terminal;

use crate::backend::{self, Services};
use crate::config::{Settings, Store};
use crate::redact::Redactor;

pub trait Prompter {
    fn message(&mut self, text: &str) -> Result<(), String>;
    fn input(&mut self, prompt: &str) -> Result<Option<String>, String>;
    fn secret(&mut self, prompt: &str) -> Result<Option<String>, String>;
}

pub fn configure(
    store: &Store,
    current: Option<&Settings>,
    ui: &mut impl Prompter,
) -> Result<Option<Settings>, String> {
    configure_with(store, current, ui, |id, key| {
        Services::for_backend(id, key.into())
    })
}

fn configure_with(
    store: &Store,
    current: Option<&Settings>,
    ui: &mut impl Prompter,
    make_services: impl Fn(&str, &str) -> Result<Services, String>,
) -> Result<Option<Settings>, String> {
    let available = backend::available();
    let backend_id = if available.len() == 1 {
        available[0].id
    } else {
        loop {
            ui.message("Available backends:")?;
            for (index, backend) in available.iter().enumerate() {
                ui.message(&format!("{}. {}", index + 1, backend.name))?;
            }
            let Some(answer) = answer(ui.input("Choose a backend number: ")?) else {
                return cancelled(ui);
            };
            if let Some(backend) = answer
                .parse::<usize>()
                .ok()
                .and_then(|number| number.checked_sub(1))
                .and_then(|index| available.get(index))
            {
                break backend.id;
            }
            ui.message("Choose one of the displayed backends.")?;
        }
    };
    let selected = available
        .iter()
        .find(|backend| backend.id == backend_id)
        .unwrap();
    let name = selected.name;
    let note = selected
        .key_url
        .map_or(String::new(), |url| format!("Get a key at {url}. "));
    ui.message(&format!("Jecode setup\nCredentials and model are saved in plain text in {}.\n{note}Type /cancel to leave setup.", store.path().display()))?;
    let (api_key, services) = loop {
        let prompt = if current.is_some() {
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
        .filter(|settings| settings.backend_id == backend_id)
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
