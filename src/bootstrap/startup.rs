use super::*;
use crate::{
    backend::{Backend, unconfigured},
    tui::StartupNotice,
};

pub(super) fn needs_setup(store: &Store, settings: &Settings) -> bool {
    !settings.has_model()
        || settings
            .services(
                store.path().parent().expect("configuration directory"),
                &settings.backend_id,
            )
            .is_err()
}

pub(super) fn client(
    store: &Store,
    settings: &Settings,
    model_override: Option<String>,
    tui: bool,
) -> Result<(Box<dyn Backend>, Option<StartupNotice>), String> {
    let home = store.path().parent().expect("configuration directory");
    if !tui && needs_setup(store, settings) {
        return Err("Jecode needs provider credentials and a default model. Run jecode setup in a terminal, then retry your task.".into());
    }
    let model = model_override
        .clone()
        .or_else(|| settings.has_model().then(|| settings.model.clone()));
    let unavailable = match settings.service_status(home, &settings.backend_id) {
        Ok(services) => {
            if let Some(model) = model {
                return services.create(model).map(|client| (client, None));
            }
            let text = "No model selected · Choose a model to finish configuration".to_string();
            return Ok((
                unconfigured::client(services.id(), None, Some(services), text.clone()),
                Some(StartupNotice {
                    text,
                    action: ("/model", "choose model"),
                }),
            ));
        }
        Err(error) => error,
    };
    let cause = &unavailable.diagnostic;
    if let Some(services) = settings.configured_services(home).into_iter().next() {
        let id = services.id();
        let name = crate::backend::available()
            .iter()
            .find(|info| info.id == id)
            .map_or(id, |info| info.name);
        let mut text =
            format!("Default provider unavailable · {cause} · Using {name} for this conversation");
        if model_override.is_some() {
            text.push_str(" · --model override skipped for the fallback provider");
        }
        if let Some(model) = store.previous_model(id)? {
            let mut client = services.create(model)?;
            client.set_routing(crate::backend::Routing::Auto);
            return Ok((
                client,
                Some(StartupNotice {
                    text,
                    action: ("/settings", "providers"),
                }),
            ));
        }
        text.push_str(" · Choose a model");
        let client = unconfigured::client(id, None, Some(services), text.clone());
        return Ok((
            client,
            Some(StartupNotice {
                text,
                action: ("/model", "choose model"),
            }),
        ));
    }
    let text = if !settings.has_model()
        && settings.credentials.is_empty()
        && unavailable.settings.is_some()
    {
        "No configured provider".into()
    } else {
        format!("No configured provider · {cause}")
    };
    Ok((
        unconfigured::client(&settings.backend_id, model, None, text.clone()),
        Some(StartupNotice {
            text,
            action: ("/settings", "configure provider"),
        }),
    ))
}

#[cfg(test)]
mod tests;
