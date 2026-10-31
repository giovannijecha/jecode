use crate::agent::Agent;
use crate::attachments::Prompt;
use crate::config::{self, Settings, Store};
use crate::console::Console;
use crate::session::{self, SessionConfig};
use crate::{setup, tools::Tools};
use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;

mod startup;

pub fn configure() -> Result<(), String> {
    require_terminal()?;
    let store = Store::discover()?;
    let settings = store.load()?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    setup::configure(
        &store,
        settings.as_ref(),
        &mut setup::Terminal::new(&mut input, &mut output),
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

/// `jecode checkpoints ...`: reads no provider settings and needs no Bash.
pub fn checkpoints(words: &[String]) -> Result<(), String> {
    let store = Store::discover()?;
    let home = store.path().parent().expect("configuration directory");
    let root =
        std::env::current_dir().map_err(|error| format!("Invalid working directory: {error}"))?;
    let mut stdout = io::stdout().lock();
    if crate::checkpoints::command::stream_show(home, &root, words, &mut stdout)? {
        return stdout.flush().map_err(|error| error.to_string());
    }
    let output = crate::checkpoints::command::offline(home, &root, words)?;
    let written = match &output.raw {
        Some(raw) => stdout.write_all(raw.as_bytes()),
        None => writeln!(stdout, "{}", output.text()),
    };
    written
        .and_then(|()| stdout.flush())
        .map_err(|error| format!("Could not print checkpoints: {error}"))
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
    let tui = interactive && !plain && prompt.is_none() && io::stdout().is_terminal();
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
    let settings = if tui && settings.is_none() {
        Some(Settings::unconfigured())
    } else {
        settings
    };
    if settings.is_none() && !interactive {
        return Err(
            "Jecode is not configured. Run jecode setup in a terminal, then retry your task."
                .into(),
        );
    }
    let directory = std::env::current_dir().map_err(|error| error.to_string())?;
    let tools = Tools::new(&directory)?;
    let mut output = io::stdout().lock();
    let mut status = io::stderr().lock();
    let mut input = io::stdin().lock();
    let needs_setup = settings
        .as_ref()
        .is_none_or(|settings| !tui && interactive && startup::needs_setup(&store, settings));
    let settings = if needs_setup {
        let Some(settings) = setup::configure(
            &store,
            settings.as_ref(),
            &mut setup::Terminal::new(&mut input, &mut output),
        )?
        else {
            return Ok(());
        };
        settings
    } else {
        settings.unwrap()
    };
    let config_home = store.path().parent().expect("configuration directory");
    let (mut client, notice) = startup::client(&store, &settings, model_override, tui)?;
    if client.id() == settings.backend_id && client.model() == settings.model {
        client.set_effort(settings.effort);
    }
    let mut agent = Agent::with_backend(client, tools);
    agent.include_credentials(account_redactor(config_home, &mut status)?);
    for services in settings.configured_services(config_home) {
        agent.register_backend(services);
    }
    agent.set_auto_compact(settings.auto_compact);
    agent
        .permissions()
        .set_default(settings.default_permissions);
    agent.set_backend_routing_preferences(&settings.backend_id, settings.routing.clone());
    if agent.backend_id() == settings.backend_id {
        agent.set_routing(settings.routing_for(agent.model()));
    }
    if let Err(error) = std::fs::create_dir_all(config_home.join("skills")) {
        // The optional library must not prevent ordinary work. Its scan also
        // reports unreadable or malformed entries in the catalog.
        eprintln!("Could not create personal skills library: {error}");
    }
    agent.enable_sessions(config_home)?;
    agent.name_sessions();
    agent.enable_checkpoints(config_home);
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
    let mut session_config = SessionConfig { store, settings };
    if !plain && io::stdout().is_terminal() {
        drop(input);
        drop(output);
        drop(status);
        return crate::tui::launch(
            agent,
            session_config,
            resume,
            Prompt::new(String::new(), attachments),
            notice,
        );
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

fn account_redactor(
    home: &Path,
    status: &mut impl Write,
) -> Result<crate::redact::Redactor, String> {
    let mut redactor = crate::redact::Redactor::empty();
    for backend in crate::backend::available()
        .iter()
        .filter(|backend| crate::auth::account_backend(backend.id))
    {
        let (account_redactor, invalid) =
            crate::auth::AccountStore::new(home.to_path_buf(), backend.id).redactor_report()?;
        redactor.include(account_redactor);
        if invalid > 0 {
            writeln!(
                status,
                "Skipped {invalid} invalid {} account record(s); inspect the saved accounts.",
                backend.name
            )
            .map_err(|error| {
                format!(
                    "Could not report invalid {} accounts: {error}",
                    backend.name
                )
            })?;
        }
    }
    Ok(redactor)
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

    #[test]
    fn openrouter_startup_reports_corrupt_inactive_account_and_keeps_valid_redaction() {
        let directory = Directory::new();
        let home = directory.path();
        let settings =
            Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
        assert!(settings.services(home, "openrouter").is_ok());
        crate::auth::openai::Store::fixture_account(home.to_path_buf(), "valid-access-token")
            .unwrap();
        fs::write(home.join("accounts/openai/account-bad.json"), "{broken").unwrap();
        let mut status = Vec::new();
        let redactor = account_redactor(home, &mut status).unwrap();
        assert_eq!(redactor.text("valid-access-token"), "[redacted]");
        assert!(
            String::from_utf8(status)
                .unwrap()
                .contains("Skipped 1 invalid OpenAI account")
        );
    }

    #[test]
    fn startup_masks_both_subscription_providers_when_openrouter_is_selected() {
        let directory = Directory::new();
        let home = directory.path();
        crate::auth::openai::Store::fixture_account(home.to_path_buf(), "isolated-chatgpt-token")
            .unwrap();
        crate::auth::anthropic::Store::fixture_account(home.to_path_buf(), "isolated-claude-token")
            .unwrap();
        let redactor = account_redactor(home, &mut Vec::new()).unwrap();
        assert_eq!(
            redactor.text("isolated-chatgpt-token isolated-claude-token"),
            "[redacted] [redacted]"
        );
    }
}
