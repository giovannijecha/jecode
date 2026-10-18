use crate::{
    agent::Agent,
    config::{Settings, Store},
    effort::Effort,
    openrouter::{Model, OpenRouter, routing},
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
            let mut client = OpenRouter::with_api(
                agent.api().with_key(settings.api_key.clone())?,
                agent.model().into(),
            )?;
            client.set_effort(agent.effort());
            agent.replace_client(client);
            agent.set_auto_compact(settings.auto_compact);
            agent.set_routing_preferences(settings.routing.clone());
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
        let api = agent.api();
        let mut ui = Terminal::new(input, output, &self.bash);
        let id = if let Some(id) = id {
            crate::openrouter::validate_model(id)?;
            id.into()
        } else {
            let Some(id) = setup::choose_model(&api, Some(agent.model()), &mut ui)? else {
                return Ok(());
            };
            id
        };
        let catalog = api.catalog()?;
        let model = catalog
            .iter()
            .find(|model| model.id == id)
            .ok_or_else(|| format!("Model not found or does not support tools: {id}"))?;
        let Some(effort) = choose_effort(model, &mut ui)? else {
            return Ok(());
        };
        let previous = format!("{} · {}", agent.model(), agent.effort().name());
        agent.set_model(id)?;
        agent.set_effort(effort);
        let result = format!(
            "Model set to {} · {} · was {previous}",
            agent.model(),
            effort.name()
        );
        agent.record_local("/model", &result);
        writeln!(output, "{result}").map_err(|error| error.to_string())
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
        let fresh = agent
            .router()
            .endpoints(&model)
            .filter(|endpoints| endpoints.fresh(std::time::Instant::now()))
            .cloned();
        let listing = match fresh {
            Some(endpoints) => Ok(endpoints),
            None => agent.api().endpoints(&model),
        };
        let endpoints = match listing {
            Ok(endpoints) => {
                agent.offer_endpoints(endpoints.clone());
                Some(endpoints)
            }
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
        let entries = routing::entries(
            endpoints.as_ref(),
            agent.routing(),
            agent.effort().reasons(),
        );
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
        let effort = if requested == Some(Effort::Default) {
            Effort::Default
        } else {
            let catalog = agent.api().catalog()?;
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
