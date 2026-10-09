use crate::agent::Agent;
use crate::attachments::Prompt;
use crate::config::{self, Settings, Store};
use crate::console::Console;
use crate::openrouter::OpenRouter;
use crate::session::{self, SessionConfig};
use crate::{setup, tools::Tools};
use std::io::{self, IsTerminal, Read};

pub fn configure() -> Result<(), String> {
    require_terminal()?;
    let store = Store::discover()?;
    let settings = store.load()?;
    let bash = crate::tools::find_bash()?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    setup::configure(
        &store,
        settings.as_ref(),
        &mut setup::Terminal::new(&mut input, &mut output, &bash),
    )?;
    Ok(())
}

pub fn run(
    model_override: Option<String>,
    prompt: Option<String>,
    attachments: Vec<String>,
    plain: bool,
) -> Result<(), String> {
    launch(model_override, prompt, attachments, plain, None)
}

pub fn resume(id: Option<String>, plain: bool) -> Result<(), String> {
    launch(None, None, Vec::new(), plain, Some(id))
}

fn launch(
    model_override: Option<String>,
    prompt: Option<String>,
    attachments: Vec<String>,
    plain: bool,
    resume: Option<Option<String>>,
) -> Result<(), String> {
    let store = Store::discover()?;
    let interactive = io::stdin().is_terminal();
    let settings = resolve_settings(
        &store,
        std::env::var("OPENROUTER_API_KEY").ok(),
        std::env::var("OPENROUTER_MODEL")
            .ok()
            .or_else(|| model_override.clone()),
    );
    let settings = match settings {
        Ok(settings) => settings,
        // A partial legacy environment should not obstruct a normal first launch.
        Err(error) if interactive && store.load()?.is_none() => {
            eprintln!("{error}");
            None
        }
        Err(error) => return Err(error),
    };
    if settings.is_none() && !interactive {
        return Err(
            "Jecode is not configured. Run jecode setup in a terminal, then retry your task."
                .into(),
        );
    }
    let directory = std::env::current_dir().map_err(|error| error.to_string())?;
    let tools = Tools::new(&directory)?;
    let bash = tools.bash().to_path_buf();
    let mut output = io::stdout().lock();
    let mut status = io::stderr().lock();
    let mut input = io::stdin().lock();
    let settings = match settings {
        Some(settings) => settings,
        None => {
            let Some(settings) = setup::configure(
                &store,
                None,
                &mut setup::Terminal::new(&mut input, &mut output, &bash),
            )?
            else {
                return Ok(());
            };
            settings
        }
    };
    let model = model_override.unwrap_or_else(|| settings.model.clone());
    let mut client = OpenRouter::new(settings.api_key.clone(), model)?;
    if client.model() == settings.model {
        client.set_effort(settings.effort);
    }
    let mut agent = Agent::new(client, tools);
    agent.enable_sessions(store.path().parent().expect("configuration directory"))?;
    let attachments = agent.import_attachments(
        &attachments
            .iter()
            .map(|path| crate::attachments::paths::argument(path, &directory))
            .collect::<Vec<_>>(),
    )?;
    if let Some(prompt) = prompt {
        return agent.run_turn(
            Prompt::new(prompt, attachments),
            &mut Console::new(&mut output, &mut status),
        );
    }
    if !interactive {
        if let Some(resume) = resume {
            let id =
                resume.ok_or("Choosing a session needs interactive input; supply a session ID")?;
            agent.resume(&id)?;
        }
        let mut prompt = String::new();
        input
            .take(1024 * 1024 + 1)
            .read_to_string(&mut prompt)
            .map_err(|error| format!("Could not read task: {error}"))?;
        if prompt.len() > 1024 * 1024 {
            return Err("Task exceeds the 1 MiB input limit".into());
        }
        return agent.run_turn(
            Prompt::new(prompt, attachments),
            &mut Console::new(&mut output, &mut status),
        );
    }
    let mut session_config = SessionConfig {
        store,
        settings,
        bash,
    };
    if !plain && io::stdout().is_terminal() {
        drop(input);
        drop(output);
        drop(status);
        return match resume {
            Some(id) => crate::tui::resume(agent, session_config, id),
            None => crate::tui::run(
                agent,
                session_config,
                Prompt::new(String::new(), attachments),
            ),
        };
    }
    match resume {
        Some(id) => session::chat_with_resume(
            &mut agent,
            &mut session_config,
            &mut input,
            &mut output,
            &mut status,
            id,
        ),
        None => session::chat(
            &mut agent,
            &mut session_config,
            &mut input,
            &mut output,
            &mut status,
            attachments,
        ),
    }
}

fn require_terminal() -> Result<(), String> {
    if !io::stdin().is_terminal() {
        return Err(
            "Setup needs interactive terminal input. Run jecode setup directly in your terminal."
                .into(),
        );
    }
    Ok(())
}

fn resolve_settings(
    store: &Store,
    key: Option<String>,
    model: Option<String>,
) -> Result<Option<Settings>, String> {
    match store.load()? {
        Some(settings) => Ok(Some(settings)),
        None => config::environment_settings(key, model),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;
    use std::fs;

    #[test]
    fn saved_settings_win_over_environment_and_invalid_files_do_not_fall_back() {
        let directory = Directory::new();
        let store = Store::new(directory.path().to_path_buf());
        let settings =
            Settings::new("isolated-fixture-key".into(), "fixture/saved".into()).unwrap();
        store.save(&settings).unwrap();
        let resolved = resolve_settings(
            &store,
            Some("stale-fixture-key".into()),
            Some("fixture/old".into()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(resolved.api_key, settings.api_key);
        assert_eq!(resolved.model, "fixture/saved");
        fs::write(store.path(), "{broken").unwrap();
        assert!(
            resolve_settings(
                &store,
                Some("stale-fixture-key".into()),
                Some("fixture/old".into())
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(store.path()).unwrap(), "{broken");
    }
}
