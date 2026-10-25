use crate::{
    agent::Agent,
    backend::{self, Model},
    config::{Settings, Store},
    effort::Effort,
    setup::{self, Prompter, Terminal},
};
use std::io::{BufRead, Write};
use std::path::PathBuf;

pub struct SessionConfig {
    pub store: Store,
    pub settings: Settings,
    pub bash: PathBuf,
}

impl SessionConfig {
    pub fn configure(
        &mut self,
        agent: &mut Agent,
        input: &mut impl BufRead,
        output: &mut impl Write,
    ) -> Result<(), String> {
        let mut ui = Terminal::new(input, output, &self.bash);
        if let Some(settings) = setup::configure(&self.store, Some(&self.settings), &mut ui)? {
            let home = self.store.path().parent().expect("configuration directory");
            for services in settings.configured_services(home) {
                agent.register_backend(services);
            }
            if agent.backend_id() == settings.backend_id {
                let mut client = settings
                    .services(home, &settings.backend_id)?
                    .create(agent.model().into())?;
                client.set_effort(agent.effort());
                agent.replace_backend(client)?;
            }
            agent.set_auto_compact(settings.auto_compact);
            agent.set_backend_routing_preferences(&settings.backend_id, settings.routing.clone());
            self.settings = settings;
            writeln!(
                output,
                "Defaults saved. Current conversation and model kept."
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn change_model(
        &mut self,
        agent: &mut Agent,
        id: Option<&str>,
        input: &mut impl BufRead,
        output: &mut impl Write,
    ) -> Result<(), String> {
        let mut services = agent.services();
        let mut ui = Terminal::new(input, output, &self.bash);
        let manual = id.is_some();
        if !manual {
            let home = self.store.path().parent().expect("configuration directory");
            let configured = self.settings.configured_services(home);
            if configured.len() > 1 {
                ui.message("Providers:")?;
                for (index, provider) in configured.iter().enumerate() {
                    let name = backend::available()
                        .iter()
                        .find(|info| info.id == provider.id())
                        .map_or(provider.id(), |info| info.name);
                    ui.message(&format!("{}. {name}", index + 1))?;
                }
                let Some(value) =
                    ui.input("Choose a provider number (/cancel leaves unchanged): ")?
                else {
                    return Ok(());
                };
                if value.trim() == "/cancel" || value.trim().is_empty() {
                    return Ok(());
                }
                let provider = value
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .and_then(|index| configured.get(index))
                    .ok_or("Choose one of the displayed providers")?;
                if provider.id() != services.id() || provider.account() != services.account() {
                    services = provider.clone();
                }
            }
        }
        let id = if let Some(id) = id {
            crate::config::validate_model(id)?;
            id.into()
        } else {
            let current = (services.id() == agent.backend_id()
                && services.account() == agent.identity().account)
                .then(|| agent.model());
            let Some(id) = setup::choose_model(&services, current, &mut ui)? else {
                return Ok(());
            };
            id
        };
        let effort = match (!manual)
            .then(|| services.catalog().ok())
            .flatten()
            .and_then(|catalog| catalog.into_iter().find(|model| model.id == id))
        {
            Some(model) => match choose_effort(&model, &mut ui)? {
                Some(effort) => effort,
                None => return Ok(()),
            },
            None => Effort::Default,
        };
        let previous = format!("{} · {}", agent.model(), agent.effort().name());
        if services.id() == agent.backend_id() && services.account() == agent.identity().account {
            agent.set_model(id)?;
            agent.set_effort(effort);
        } else {
            let mut client = services.create(id)?;
            client.set_effort(effort);
            agent.replace_backend(client)?;
        }
        let result = format!(
            "Model set to {} · {} · was {previous}",
            agent.model(),
            effort.name()
        );
        agent.record_local("/model", &result);
        writeln!(output, "{result}").map_err(|error| error.to_string())
    }

    pub fn change_backend(
        &mut self,
        agent: &mut Agent,
        argument: &str,
        input: &mut impl BufRead,
        output: &mut impl Write,
    ) -> Result<(), String> {
        let mut ui = Terminal::new(input, output, &self.bash);
        let (backend_id, model) = match argument.split_once(char::is_whitespace) {
            Some((id, model)) => (id.trim().to_string(), Some(model.trim().to_string())),
            None if !argument.is_empty() => (argument.into(), None),
            None => {
                ui.message("Available backends:")?;
                for (index, backend) in backend::available().iter().enumerate() {
                    ui.message(&format!("{}. {}", index + 1, backend.name))?;
                }
                let Some(answer) = ui.input("Choose a backend number (Enter cancels): ")? else {
                    return Ok(());
                };
                let Some(info) = answer
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|number| number.checked_sub(1))
                    .and_then(|index| backend::available().get(index))
                else {
                    return Ok(());
                };
                (info.id.into(), None)
            }
        };
        if !backend::available()
            .iter()
            .any(|info| info.id == backend_id)
        {
            return Err(format!("Backend is not available: {backend_id}"));
        }
        let services = if backend_id == agent.backend_id() {
            agent.services()
        } else {
            self.settings.services(
                self.store.path().parent().expect("configuration directory"),
                &backend_id,
            )?
        };
        let manual = model.is_some();
        let model = match model {
            Some(model) => {
                crate::config::validate_model(&model)?;
                model
            }
            None => match setup::choose_model(
                &services,
                (backend_id == agent.backend_id()).then(|| agent.model()),
                &mut ui,
            )? {
                Some(model) => model,
                None => return Ok(()),
            },
        };
        let effort = match (!manual)
            .then(|| services.catalog().ok())
            .flatten()
            .and_then(|models| models.into_iter().find(|entry| entry.id == model))
        {
            Some(model) => match choose_effort(&model, &mut ui)? {
                Some(effort) => effort,
                None => return Ok(()),
            },
            None => Effort::Default,
        };
        let previous = format!("{} / {}", agent.backend_id(), agent.model());
        if services.id() == agent.backend_id() && services.account() == agent.identity().account {
            agent.set_model(model.clone())?;
            agent.set_effort(effort);
        } else {
            let mut client = services.create(model.clone())?;
            client.set_effort(effort);
            agent.replace_backend(client)?;
        }
        let result = format!("Backend set to {backend_id} / {model} · was {previous}");
        agent.record_local("/backend", &result);
        ui.message(&result)
    }

    /// Chooses this conversation's routing from the shared entries; saved
    /// defaults are unchanged. Without a listing, Auto and the current choice remain.
    pub fn change_routing(
        &mut self,
        agent: &mut Agent,
        input: &mut impl BufRead,
        output: &mut impl Write,
    ) -> Result<(), String> {
        let model = agent.model().to_owned();
        let fresh = agent.cached_endpoints(&model);
        let listing = match fresh {
            Some(endpoints) => Ok(Some(endpoints)),
            None => agent.services().endpoints(&model),
        };
        let endpoints = match listing {
            Ok(Some(endpoints)) => {
                agent.offer_endpoints(endpoints.clone());
                Some(endpoints)
            }
            Ok(None) => return Err("Endpoint routing is unavailable for this backend".into()),
            Err(error) => {
                writeln!(
                    output,
                    "Endpoint listing unavailable: {}",
                    agent.redact(&error)
                )
                .map_err(|error| error.to_string())?;
                None
            }
        };
        let entries = agent
            .services()
            .routing_entries(
                endpoints.as_ref(),
                &agent.routing(),
                agent.effort().reasons(),
            )
            .ok_or("Endpoint routing is unavailable for this backend")?;
        let mut ui = Terminal::new(input, output, &self.bash);
        ui.message(&format!("Routing for {model}"))?;
        for (index, entry) in entries.iter().enumerate() {
            ui.message(&format!(
                "{}. {} · {}",
                index + 1,
                entry.name,
                entry.description
            ))?;
        }
        if endpoints.is_some() {
            ui.message("Prices are listed $/M input/output after discounts; speed and latency are 30-minute medians.")?;
        }
        if let Some(served) = agent.charges().describe() {
            ui.message(&format!("Last turn: {}", agent.redact(&served)))?;
        }
        let entry = loop {
            let Some(value) = ui.input("Choose a number (Enter or /cancel leaves unchanged): ")?
            else {
                return Ok(());
            };
            let value = value.trim();
            if value.is_empty() || value == "/cancel" {
                return Ok(());
            }
            let Some(entry) = value
                .parse::<usize>()
                .ok()
                .and_then(|number| number.checked_sub(1))
                .and_then(|index| entries.get(index))
            else {
                ui.message("Choose one of the displayed numbers.")?;
                continue;
            };
            match &entry.problem {
                Some(problem) => ui.message(&format!("{problem}; choose another."))?,
                None => break entry,
            }
        };
        ui.message(&entry.help)?;
        let previous = agent.routing().label();
        agent.set_routing(entry.routing.clone());
        let result = format!(
            "Routing set to {} · was {previous} · applies from the next request",
            entry.routing.label()
        );
        agent.record_local("/routing", &result);
        writeln!(output, "{result}").map_err(|error| error.to_string())
    }

    pub fn change_effort(
        &mut self,
        agent: &mut Agent,
        argument: Option<&str>,
        input: &mut impl BufRead,
        output: &mut impl Write,
    ) -> Result<(), String> {
        let requested = argument.map(Effort::parse).transpose()?;
        let effort = if let Some(effort) = requested
            .filter(|effort| *effort == Effort::Default || !agent.services().validates_effort())
        {
            effort
        } else {
            let catalog = agent.services().catalog()?;
            let model = catalog
                .iter()
                .find(|model| model.id == agent.model())
                .ok_or("Current model is not available in the tool-capable catalog")?;
            if let Some(effort) = requested {
                if !model.efforts.contains(&effort) {
                    return Err(format!(
                        "{} does not support effort {}",
                        model.id,
                        effort.name()
                    ));
                }
                effort
            } else {
                let Some(effort) =
                    choose_effort(model, &mut Terminal::new(input, output, &self.bash))?
                else {
                    return Ok(());
                };
                effort
            }
        };
        let previous = agent.effort();
        agent.set_effort(effort);
        let result = format!("Effort set to {} · was {}", effort.name(), previous.name());
        agent.record_local("/effort", &result);
        writeln!(output, "{result}").map_err(|error| error.to_string())
    }
}

fn choose_effort(model: &Model, ui: &mut impl Prompter) -> Result<Option<Effort>, String> {
    if model.efforts.len() == 1 {
        return Ok(Some(Effort::Default));
    }
    ui.message(&format!("Reasoning effort for {}", model.id))?;
    for (index, effort) in model.efforts.iter().enumerate() {
        ui.message(&format!("{}. {}", index + 1, effort.name()))?;
    }
    loop {
        let Some(value) =
            ui.input("Choose a number (Enter uses default; /cancel leaves unchanged): ")?
        else {
            return Ok(None);
        };
        let value = value.trim();
        if value == "/cancel" {
            return Ok(None);
        }
        if value.is_empty() {
            return Ok(Some(Effort::Default));
        }
        if let Ok(number) = value.parse::<usize>()
            && let Some(effort) = number
                .checked_sub(1)
                .and_then(|index| model.efforts.get(index))
        {
            return Ok(Some(*effort));
        }
        ui.message("Choose one of the displayed numbers.")?;
    }
}
