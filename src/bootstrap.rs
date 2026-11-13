use crate::agent::Agent;
use crate::attachments::Prompt;
use crate::cli::{Failure, Resume, Run, Selection, Start, usage};
use crate::config::{self, Settings, Store};
use crate::console::Console;
use crate::session::{self, SessionConfig};
use crate::{setup, tools::Tools};
use std::io::{self, IsTerminal, Write};
use std::path::Path;

mod input;
mod jsonl;
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

/// Opens the TUI or plain chat, or runs one task: a prompt argument, piped
/// stdin, `init` or `--json` make a single run that never waits for input.
pub fn run(run: Run) -> Result<(), Failure> {
    let store = Store::discover()?;
    let stdin_terminal = io::stdin().is_terminal();
    let single = run.prompt.is_some() || !stdin_terminal || run.start == Start::Init || run.json;
    if run.json && run.prompt.is_none() && stdin_terminal && run.start != Start::Init {
        return Err(usage(
            "--json needs a PROMPT or piped stdin: it prints one run as JSON Lines",
        ));
    }
    if run.start == Start::Resume(Resume::Pick) && single {
        return Err(usage(
            "choosing a conversation needs interactive input; supply SESSION_ID or --last",
        ));
    }
    // Setup runs only when someone can answer it and stdout is not JSON.
    let interactive = stdin_terminal && !run.json;
    let tui = !single && !run.plain && io::stdout().is_terminal();
    let explicit_provider = run.selection.provider.is_some();
    let (environment_key, environment_model) = if run
        .selection
        .provider
        .as_deref()
        .is_some_and(|id| id != "openrouter")
    {
        // OpenRouter environment settings cannot configure another route.
        (None, None)
    } else {
        let key = std::env::var("OPENROUTER_API_KEY").ok();
        let model = std::env::var("OPENROUTER_MODEL")
            .ok()
            .or_else(|| key.as_ref().and(run.selection.model.clone()));
        (key, model)
    };
    let settings = resolve_settings(&store, environment_key, environment_model);
    let settings = match settings {
        Ok(settings) => settings,
        // A partial legacy environment should not obstruct a normal first launch.
        Err(error) if interactive && store.load()?.is_none() => {
            eprintln!("{error}");
            None
        }
        Err(error) => return Err(error.into()),
    };
    let settings = if (tui || explicit_provider) && settings.is_none() {
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
    // Read and check piped input before any conversation is created.
    let message = if single {
        let data = if stdin_terminal {
            None
        } else {
            Some(input::read(&mut io::stdin().lock())?)
        };
        let prompt = match run.start {
            Start::Init => Some(crate::session::commands::INIT_PROMPT.to_owned()),
            _ => run.prompt.clone(),
        };
        Some(input::compose(prompt, data)?)
    } else {
        None
    };
    let directory = std::env::current_dir().map_err(|error| error.to_string())?;
    let tools = Tools::new(&directory)?;
    let mut output = io::stdout().lock();
    let mut status = io::stderr().lock();
    let mut input = io::stdin().lock();
    let needs_setup = settings.as_ref().is_none_or(|settings| {
        !tui && interactive && !explicit_provider && startup::needs_setup(&store, settings)
    });
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
    let resume = match &run.start {
        Start::Resume(Resume::Pick) => Some(None),
        Start::Resume(Resume::Last) => Some(Some(crate::selection::last_session(
            config_home,
            &directory,
        )?)),
        Start::Resume(Resume::Id(id)) => Some(Some(id.clone())),
        Start::New | Start::Init => None,
    };
    // A resumed conversation keeps its own selection; run options apply to it
    // after it opens. A new one starts from them.
    let new_selection = if resume.is_none() {
        run.selection.clone()
    } else {
        Selection::default()
    };
    // A resumed conversation replaces this client with its own, so an
    // unavailable default provider only matters for a new conversation.
    let (client, notice) =
        startup::client(&store, &settings, &new_selection, tui || resume.is_some())?;
    let mut agent = Agent::with_backend(client, tools);
    agent.include_credentials(account_redactor(config_home, &mut status)?);
    for services in settings.configured_services(config_home) {
        agent.register_backend(services);
    }
    agent.set_auto_compact(settings.auto_compact);
    agent.set_cache_retention(settings.cache_retention);
    agent
        .permissions()
        .set_default(settings.default_permissions);
    agent.set_backend_routing_preferences(&settings.backend_id, settings.routing.clone());
    if agent.backend_id() == settings.backend_id && new_selection.routing.is_none() {
        agent.set_routing(settings.routing_for(agent.model()));
    }
    if let Err(error) = std::fs::create_dir_all(config_home.join("skills")) {
        // The optional library must not prevent ordinary work. Its scan also
        // reports unreadable or malformed entries in the catalog.
        eprintln!("Could not create personal skills library: {error}");
    }
    agent.enable_sessions(config_home)?;
    agent.name_sessions();
    if resume.is_none()
        && let Some(level) = run.selection.permissions
    {
        agent.permissions().set_level(level)?;
    }
    let attachments = agent.import_attachments(
        &run.attachments
            .iter()
            .map(|path| crate::attachments::paths::argument(path, &directory))
            .collect::<Vec<_>>(),
    )?;
    if let Some(message) = message {
        agent.set_single_run();
        if let Some(Some(id)) = &resume {
            crate::selection::resume(&mut agent, id, &run.selection, &settings, &store)?;
        }
        let prompt = Prompt::new(message, attachments);
        if !run.json {
            return Ok(agent.run_turn(prompt, &mut Console::new(&mut output, &mut status))?);
        }
        let mut lines = jsonl::Lines::new(&mut output, agent.redactor());
        let result = agent.run_turn(prompt, &mut lines);
        let mut fields = vec![
            ("type", crate::json::Value::string("result")),
            (
                "status",
                crate::json::Value::string(if result.is_ok() {
                    "completed"
                } else {
                    "failed"
                }),
            ),
            (
                "session_id",
                crate::json::Value::optional(agent.sessions().map(|handle| handle.id())),
            ),
            ("provider", crate::json::Value::string(agent.backend_id())),
            ("model", crate::json::Value::string(agent.model())),
            ("effort", crate::json::Value::string(agent.effort().name())),
            (
                "permissions",
                crate::json::Value::string(agent.permissions().level().id()),
            ),
        ];
        if let Err(error) = &result {
            fields.push(("error", crate::json::Value::string(agent.redact(error))));
        }
        lines.write(crate::json::Value::entries(fields))?;
        return Ok(result?);
    }
    let mut session_config = SessionConfig { store, settings };
    if tui {
        drop(input);
        drop(output);
        drop(status);
        return Ok(crate::tui::launch(
            agent,
            session_config,
            resume,
            Prompt::new(String::new(), attachments),
            notice,
            run.selection,
        )?);
    }
    Ok(match resume {
        Some(id) => session::chat_with_resume(
            &mut agent,
            &mut session_config,
            &mut input,
            &mut output,
            &mut status,
            id,
            attachments,
            &run.selection,
        ),
        None => session::chat(
            &mut agent,
            &mut session_config,
            &mut input,
            &mut output,
            &mut status,
            attachments,
        ),
    }?)
}

pub(crate) fn account_redactor(
    home: &Path,
    status: &mut impl Write,
) -> Result<crate::redact::Redactor, String> {
    let mut redactor = crate::redact::Redactor::empty();
    if let Err(error) = crate::mcp::include_credentials(home, &redactor) {
        // Optional remote tools must not prevent ordinary work.
        writeln!(status, "{error}; continuing without MCP.")
            .map_err(|error| format!("Could not report MCP configuration: {error}"))?;
    }
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

        let retired = r#"{"selection":{"backend":"openrouter","model":"fixture/saved"},"backends":{"openrouter":{"api_key":"isolated-fixture-key"}},"openrouter":{"api_key":"retired-fixture-key"}}"#;
        fs::write(store.path(), retired).unwrap();
        let error = resolve_settings(
            &store,
            Some("stale-fixture-key".into()),
            Some("fixture/old".into()),
        )
        .err()
        .unwrap();
        assert!(error.contains("Unsupported root openrouter settings"));
        assert!(!error.contains("retired-fixture-key"));
        assert_eq!(fs::read_to_string(store.path()).unwrap(), retired);
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
    fn startup_reports_corrupt_mcp_configuration_and_continues() {
        let directory = Directory::new();
        let home = directory.path();
        fs::write(home.join("mcp.json"), "{broken").unwrap();
        let mut status = Vec::new();
        assert!(account_redactor(home, &mut status).is_ok());
        let status = String::from_utf8(status).unwrap();
        assert!(
            status.contains(&home.join("mcp.json").display().to_string()),
            "{status}"
        );
        assert!(status.contains("continuing without MCP"), "{status}");
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

    #[test]
    fn an_explicit_account_route_needs_no_saved_default() {
        let directory = Directory::new();
        let home = directory.path();
        crate::auth::openai::Store::fixture_account(home.to_path_buf(), "isolated-account-token")
            .unwrap();
        let store = Store::new(home.to_path_buf());
        let selection = Selection {
            provider: Some("openai".into()),
            model: Some("fixture/model".into()),
            ..Selection::default()
        };
        let (backend, notice) =
            startup::client(&store, &Settings::unconfigured(), &selection, false).unwrap();
        assert_eq!((backend.id(), backend.model()), ("openai", "fixture/model"));
        assert!(notice.is_none());
        assert!(!store.path().exists());
    }
}
