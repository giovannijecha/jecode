use crate::{
    agent::Agent,
    config::{Settings, Store},
    effort::Effort,
    openrouter::{Model, OpenRouter},
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
