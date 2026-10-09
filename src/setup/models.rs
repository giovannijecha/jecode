use super::{Prompter, answer};
use crate::openrouter::{Api, Model, validate_model};

pub fn choose_model(
    api: &Api,
    current: Option<&str>,
    ui: &mut impl Prompter,
) -> Result<Option<String>, String> {
    if let Some(model) = current {
        ui.message(&api.redact(&format!("Current model: {model}. Enter keeps it.")))?;
    }
    ui.message(
        "Search tool-capable models by name. Use =provider/model-id to enter a tool-capable ID directly.",
    )?;
    loop {
        let Some(query) = answer(ui.input("Model search: ")?) else {
            return Ok(None);
        };
        if query.is_empty() {
            if let Some(model) = current {
                return Ok(Some(model.into()));
            }
            ui.message("Enter a model name to search.")?;
            continue;
        }
        if let Some(id) = query.strip_prefix('=') {
            match validate_model(id) {
                Ok(()) => return Ok(Some(id.into())),
                Err(error) => {
                    ui.message(&error)?;
                    continue;
                }
            }
        }
        ui.message("Searching OpenRouter...")?;
        let models = match api.models(&query) {
            Ok(models) => models,
            Err(error) => {
                ui.message(&format!(
                    "{error}\nRetry the search or enter =provider/model-id."
                ))?;
                continue;
            }
        };
        if models.is_empty() {
            ui.message("No tool-capable models matched. Try a different name.")?;
            continue;
        }
        for (index, model) in models.iter().enumerate() {
            ui.message(&api.redact(&format!(
                "{}. {} ({}){}",
                index + 1,
                clean(&model.name),
                clean(&model.id),
                pricing(model)
            )))?;
        }
        loop {
            let Some(selection) = answer(ui.input("Choose a number (Enter searches again): ")?)
            else {
                return Ok(None);
            };
            if selection.is_empty() {
                break;
            }
            if let Ok(number) = selection.parse::<usize>()
                && let Some(model) = number.checked_sub(1).and_then(|index| models.get(index))
            {
                return Ok(Some(model.id.clone()));
            }
            ui.message("Choose one of the displayed numbers, or Enter to search again.")?;
        }
    }
}

fn clean(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

fn pricing(model: &Model) -> String {
    match (model.prompt_price, model.completion_price) {
        (Some(input), Some(output)) => {
            format!(" - ${input:.2} input / ${output:.2} output per 1M tokens")
        }
        _ => String::new(),
    }
}
