mod models;
mod terminal;

pub use models::choose_model;
pub use terminal::Terminal;

use crate::config::{Settings, Store};
use crate::openrouter::Api;

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
    configure_with(store, current, ui, |key| Api::new(key.into()))
}

fn configure_with(
    store: &Store,
    current: Option<&Settings>,
    ui: &mut impl Prompter,
    make_api: impl Fn(&str) -> Result<Api, String>,
) -> Result<Option<Settings>, String> {
    ui.message(&format!("Jecode setup\nKey and model are saved in plain text in {}.\nGet a key at https://openrouter.ai/settings/keys. Type /cancel to leave setup.", store.path().display()))?;
    let (api_key, api) = loop {
        let prompt = if current.is_some() {
            "OpenRouter API key (hidden; Enter keeps the saved key): "
        } else {
            "OpenRouter API key (hidden): "
        };
        let Some(key) = answer(ui.secret(prompt)?) else {
            return cancelled(ui);
        };
        let key = if key.is_empty() {
            current.map_or_else(String::new, |settings| settings.api_key.clone())
        } else {
            key
        };
        let api = match make_api(&key) {
            Ok(api) => api,
            Err(error) => {
                ui.message(&error)?;
                continue;
            }
        };
        ui.message("Checking the key with OpenRouter...")?;
        match api.check_key() {
            Ok(()) => break (key, api),
            Err(error) => ui.message(&format!("{error}\nTry another key, or /cancel."))?,
        }
    };
    let Some(model) = choose_model(&api, current.map(|settings| settings.model.as_str()), ui)?
    else {
        return cancelled(ui);
    };
    let mut settings = Settings::new(api_key, model)?;
    if let Some(current) = current
        && current.model == settings.model
    {
        settings.effort = current.effort;
    }
    store.save(&settings).map_err(|error| api.redact(&error))?;
    ui.message(&api.redact(&format!("Saved. Model: {}", settings.model)))?;
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
