//! Provider, model, effort and permission selection shared by command-line
//! runs, resumed conversations, plain chat and the TUI launch. Run options
//! choose for one conversation; they never change saved defaults.

use crate::agent::Agent;
use crate::backend::{Backend, Endpoints, Routing, Services};
use crate::cli::Selection;
use crate::config::{Settings, Store};
use crate::effort::Effort;
use std::path::Path;

/// Refuses a level the model's catalog does not list where the provider
/// validates effort; `/effort NAME`, `--effort` and saved defaults share this.
pub fn check_effort(services: &Services, model: &str, effort: Effort) -> Result<(), String> {
    if effort == Effort::Default || !services.validates_effort() {
        return Ok(());
    }
    let catalog = services.catalog()?;
    let entry = catalog
        .iter()
        .find(|entry| entry.id == model)
        .ok_or_else(|| format!("{model} is not available in the tool-capable catalog"))?;
    if entry.efforts.contains(&effort) {
        Ok(())
    } else {
        Err(format!("{model} does not support effort {}", effort.name()))
    }
}

/// The provider's model when only the provider is chosen: the saved default
/// when it is that provider, otherwise the provider's last saved model.
pub fn provider_model(
    settings: &Settings,
    store: &Store,
    provider: &str,
) -> Result<String, String> {
    if provider == settings.backend_id && settings.has_model() {
        return Ok(settings.model.clone());
    }
    store.previous_model(provider)?.ok_or_else(|| {
        format!("No model is saved for {provider}; choose one with --model ID (see jecode models list --provider {provider})")
    })
}

/// The provider's services, never another provider's: an explicit choice
/// does not fall back to a different billing or account route.
pub fn services(settings: &Settings, home: &Path, provider: &str) -> Result<Services, String> {
    settings.service_status(home, provider).map_err(|error| {
        let info = crate::backend::info(provider);
        let hint = if info.is_some_and(|info| info.uses_api_key()) {
            format!("save a key with jecode providers login {provider}")
        } else {
            format!("sign in with jecode providers login {provider}")
        };
        format!("{}; {hint}", error.diagnostic)
    })
}

pub fn check_routing_provider(provider: &str, routing: Option<&Routing>) -> Result<(), String> {
    if routing.is_some() && provider != "openrouter" {
        return Err(format!(
            "--routing is only available with openrouter, not {provider}"
        ));
    }
    Ok(())
}

/// Validate an explicit pin against the provider's current choices. Auto does
/// not need a listing. The inventory can be offered to the client afterward.
pub fn check_routing(
    services: &Services,
    model: &str,
    effort: Effort,
    routing: &Routing,
) -> Result<Option<Endpoints>, String> {
    check_routing_provider(services.id(), Some(routing))?;
    let Routing::Endpoint(tag) = routing else {
        return if *routing == Routing::Auto {
            Ok(None)
        } else {
            Err("--routing needs auto or an endpoint tag".into())
        };
    };
    let endpoints = services
        .endpoints(model)
        .map_err(|error| services.redact(&error))?
        .ok_or("Endpoint routing is unavailable for openrouter")?;
    let entry = services
        .routing_entries(Some(&endpoints), &Routing::Auto, effort.reasons())
        .and_then(|entries| entries.into_iter().find(|entry| entry.routing == *routing))
        .ok_or_else(|| format!("{model} has no endpoint '{tag}'; choose one with /routing"))?;
    if let Some(problem) = entry.problem {
        return Err(problem);
    }
    Ok(Some(endpoints))
}

/// The selection for a new conversation, from run options over saved defaults.
pub struct Chosen {
    pub provider: String,
    pub model: String,
    pub effort: Effort,
}

pub fn for_new(
    settings: &Settings,
    store: &Store,
    selection: &Selection,
) -> Result<Chosen, String> {
    let provider = selection
        .provider
        .clone()
        .unwrap_or_else(|| settings.backend_id.clone());
    let model = match &selection.model {
        Some(model) => model.clone(),
        None => provider_model(settings, store, &provider)?,
    };
    let effort = selection.effort.unwrap_or(
        if provider == settings.backend_id && model == settings.model {
            settings.effort
        } else {
            Effort::Default
        },
    );
    Ok(Chosen {
        provider,
        model,
        effort,
    })
}

/// Build the explicitly selected backend without touching saved defaults.
pub fn explicit_backend(
    settings: &Settings,
    store: &Store,
    selection: &Selection,
) -> Result<Box<dyn Backend>, String> {
    let chosen = for_new(settings, store, selection)?;
    check_routing_provider(&chosen.provider, selection.routing.as_ref())?;
    let home = store.path().parent().expect("configuration directory");
    let services = services(settings, home, &chosen.provider)?;
    if selection.effort.is_some() {
        check_effort(&services, &chosen.model, chosen.effort)?;
    }
    let mut client = services.create(chosen.model)?;
    client.set_effort(chosen.effort);
    if let Some(routing) = &selection.routing {
        if let Some(endpoints) = check_routing(&services, client.model(), client.effort(), routing)?
        {
            client.offer_endpoints(endpoints);
        }
        client.set_routing(routing.clone());
    }
    Ok(client)
}

/// Restore a saved conversation and apply launch options before its UI or next
/// turn starts. An explicit provider can replace unavailable saved credentials.
pub fn resume(
    agent: &mut Agent,
    id: &str,
    selection: &Selection,
    settings: &Settings,
    store: &Store,
) -> Result<String, String> {
    let replacement = if selection.provider.is_some() {
        let current = agent.sessions().ok_or("Session storage is unavailable")?;
        let saved = current.store().read(id)?;
        if let Some(model) = saved
            .model
            .as_ref()
            .filter(|_| !agent.has_backend(&saved.backend))
        {
            let was = format!("{} / {model} · {}", saved.backend, saved.effort.name());
            let mut prepared = selection.clone();
            // Changing provider resets effort unless this run chooses one.
            prepared.effort.get_or_insert(Effort::Default);
            let mut client = explicit_backend(settings, store, &prepared)?;
            if selection.routing.is_none() {
                let routing = if client.id() == settings.backend_id {
                    settings.routing_for(client.model())
                } else {
                    Routing::Auto
                };
                client.set_routing(routing);
            }
            Some((client, (was, saved.routing)))
        } else {
            None
        }
    } else {
        None
    };
    let (replacement, replaced) = replacement.unzip();
    let status = agent.resume_with_backend(id, replacement)?;
    let summary = apply(agent, selection, settings, store, replaced)?;
    Ok(match summary {
        Some(summary) => format!("{status} {summary}"),
        None => status,
    })
}

/// Applies run options to an open conversation, typically a resumed one. Its
/// own provider, model, routing, effort and level stay unless an option names them.
/// Returns a description of the change for the conversation record.
/// `replaced` describes the saved selection that a prepared replacement
/// backend took over; that backend was validated when it was built.
fn apply(
    agent: &mut Agent,
    selection: &Selection,
    settings: &Settings,
    store: &Store,
    replaced: Option<(String, Routing)>,
) -> Result<Option<String>, String> {
    if selection.is_empty() {
        return Ok(None);
    }
    let home = store.path().parent().expect("configuration directory");
    let prepared_replacement = replaced.is_some();
    let (previous, previous_routing) = replaced.unwrap_or_else(|| {
        (
            format!(
                "{} / {} · {}",
                agent.backend_id(),
                agent.model(),
                agent.effort().name()
            ),
            agent.routing(),
        )
    });
    let mut changes = Vec::new();
    let provider = selection.provider.as_deref().unwrap_or(agent.backend_id());
    check_routing_provider(provider, selection.routing.as_ref())?;
    let current_routing_endpoints = if selection.provider.is_none() && selection.model.is_none() {
        selection
            .routing
            .as_ref()
            .map(|routing| {
                check_routing(
                    &agent.services(),
                    agent.model(),
                    selection.effort.unwrap_or(agent.effort()),
                    routing,
                )
            })
            .transpose()?
            .flatten()
    } else {
        None
    };
    if selection.provider.is_some() || selection.model.is_some() {
        let provider = selection
            .provider
            .clone()
            .unwrap_or_else(|| agent.backend_id().to_owned());
        let same_provider = provider == agent.backend_id();
        let model = match &selection.model {
            Some(model) => model.clone(),
            None if same_provider => agent.model().to_owned(),
            None => provider_model(settings, store, &provider)?,
        };
        let effort = selection
            .effort
            .unwrap_or(if same_provider && model == agent.model() {
                agent.effort()
            } else {
                Effort::Default
            });
        let services = if same_provider {
            agent.services()
        } else {
            services(settings, home, &provider)?
        };
        if !prepared_replacement {
            check_effort(&services, &model, effort)?;
        }
        let endpoints = if prepared_replacement {
            None
        } else {
            selection
                .routing
                .as_ref()
                .map(|routing| check_routing(&services, &model, effort, routing))
                .transpose()?
                .flatten()
        };
        if same_provider {
            agent.set_model(model)?;
            agent.set_effort(effort);
        } else {
            let mut client = services.create(model)?;
            client.set_effort(effort);
            if let Some(endpoints) = &endpoints {
                client.offer_endpoints(endpoints.clone());
            }
            agent.register_backend(services);
            if let Some(routing) = &selection.routing {
                agent.replace_backend_with_routing(client, routing.clone())?;
            } else {
                agent.replace_backend(client)?;
            }
        }
        if let Some(endpoints) = endpoints {
            agent.offer_endpoints(endpoints);
        }
        changes.push(format!(
            "Model set to {} / {} · {} · was {previous}",
            agent.backend_id(),
            agent.model(),
            agent.effort().name()
        ));
    } else if let Some(effort) = selection.effort {
        check_effort(&agent.services(), agent.model(), effort)?;
        let was = agent.effort();
        agent.set_effort(effort);
        changes.push(format!(
            "Effort set to {} · was {}",
            effort.name(),
            was.name()
        ));
    }
    if let Some(endpoints) = current_routing_endpoints {
        agent.offer_endpoints(endpoints);
    }
    if let Some(routing) = &selection.routing {
        agent.set_routing(routing.clone());
        changes.push(format!(
            "Routing set to {} · was {}",
            routing.label(),
            previous_routing.label()
        ));
    }
    if let Some(level) = selection.permissions {
        let was = agent.permissions().level();
        agent.permissions().set_level(level)?;
        changes.push(format!(
            "Permissions set to {} · was {}",
            level.name(),
            was.name()
        ));
    }
    let summary = changes.join(" · ");
    agent.record_local("Run options", &summary);
    Ok(Some(summary))
}

/// The most recent saved conversation of `directory`, read without opening it.
pub fn last_session(home: &Path, directory: &Path) -> Result<String, String> {
    crate::sessions::Store::new(home.to_path_buf(), directory)?
        .list()?
        .sessions
        .first()
        .map(|session| session.id.clone())
        .ok_or_else(|| "No saved conversations in this folder; start one with jecode".into())
}

#[cfg(test)]
mod tests;
