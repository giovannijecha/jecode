use super::*;
use crate::backend::{Backend, unconfigured};

pub(crate) type Notice = (String, &'static str, &'static str);

pub(super) fn client(
    store: &Store,
    settings: &Settings,
    model_override: Option<String>,
    tui: bool,
) -> Result<(Box<dyn Backend>, Option<Notice>), String> {
    let home = store.path().parent().expect("configuration directory");
    let model = model_override.unwrap_or_else(|| settings.model.clone());
    match settings.services(home, &settings.backend_id) {
        Ok(services) => {
            if model == "unconfigured" {
                let message = "No model selected · Choose a model with /model".to_string();
                return Ok((
                    unconfigured::client(services.id(), model, Some(services), message.clone()),
                    Some((message, "/model", "choose model")),
                ));
            }
            return services.create(model).map(|client| (client, None));
        }
        Err(error) if !tui => return Err(error),
        Err(_) => {}
    }
    if let Some(services) = settings.configured_services(home).into_iter().next() {
        let id = services.id();
        if let Some(model) = store.previous_model(id)? {
            let mut client = services.create(model)?;
            client.set_routing(crate::backend::Routing::Auto);
            return Ok((
                client,
                Some((
                    format!("Default provider unavailable · Using {id} for this conversation"),
                    "/settings",
                    "providers",
                )),
            ));
        }
        let message =
            format!("Default provider unavailable · Using {id} · Choose a model with /model");
        let client =
            unconfigured::client(id, "unconfigured".into(), Some(services), message.clone());
        return Ok((client, Some((message, "/model", "choose model"))));
    }
    let message = "No configured provider · Connect an account or add an API key".to_string();
    Ok((
        unconfigured::client(&settings.backend_id, model, None, message.clone()),
        Some((message, "/settings", "configure provider")),
    ))
}

#[cfg(test)]
mod tests;
