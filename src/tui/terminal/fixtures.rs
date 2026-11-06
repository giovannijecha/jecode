//! Conversations both native harnesses run. Each starts the real TUI, which the
//! parent drives through a ConPTY or a Unix PTY, and then checks what was saved.

use super::Terminal;
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
    json::Value,
    permissions::Level,
    session::SessionConfig,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::{
    io::{self, Write},
    path::Path,
};

fn strings(values: &[&str]) -> Value {
    Value::Array(values.iter().copied().map(Value::string).collect())
}

fn isolated_config(path: &Path) -> SessionConfig {
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let config = SessionConfig {
        store: Store::new(path.join("home")),
        settings,
    };
    config.store.save(&config.settings).unwrap();
    config
}

pub(super) fn permissions(path: &Path) {
    let outside = Directory::new();
    let file = outside.path().join("outside-note.txt");
    std::fs::write(&file, "native outside fixture\n").unwrap();
    let read = |id| {
        tool_call(
            id,
            "read",
            Value::object([("path", Value::string(file.to_string_lossy()))]),
        )
    };
    let script = (1..=40)
        .map(|step| format!("echo native-step-{step:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (executable, args) = if cfg!(windows) {
        ("powershell.exe", strings(&[&script]))
    } else {
        ("/bin/sh", strings(&["-c", &script]))
    };
    let long = tool_call(
        "native-long",
        "exec",
        Value::object([
            ("executable", Value::string(executable)),
            ("args", args),
            ("cwd", Value::string(outside.path().to_string_lossy())),
        ]),
    );
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![read("native-read-1")])),
        (200, completion("Native first read complete.", vec![])),
        (200, completion("", vec![read("native-read-2")])),
        (200, completion("Native second read complete.", vec![])),
        (200, completion("", vec![long])),
        (200, completion("Native long command denied.", vec![])),
    ]);
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(path).unwrap(),
    );
    crate::tui::launch(agent, isolated_config(path), None, Default::default(), None).unwrap();
    assert_eq!(
        Store::new(path.join("home"))
            .load()
            .unwrap()
            .unwrap()
            .default_permissions,
        Level::Full
    );
    let store = crate::sessions::Store::new(path.join("home"), path).unwrap();
    let sessions = store.list().unwrap().sessions;
    assert_eq!(sessions.len(), 1);
    let saved = store.fixture_load(&sessions[0].id).unwrap();
    assert_eq!(
        crate::permissions::replay(&saved.events),
        (Some(Level::Workspace), vec![])
    );
    assert_eq!(saved.input.draft.text, "kept draft");
    assert!(
        saved.events.iter().any(|event| {
            event.get("type").and_then(Value::as_str) == Some("permission_revoke")
        })
    );
    let tool_result = |id| {
        let content = saved
            .messages
            .iter()
            .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .unwrap();
        crate::json::parse(content).unwrap()
    };
    assert!(
        tool_result("native-read-1")
            .encode()
            .contains("native outside fixture")
    );
    for denied in ["native-read-2", "native-long"] {
        assert_eq!(
            tool_result(denied).get("outcome").and_then(Value::as_str),
            Some("denied")
        );
    }
    assert_eq!(fixture.finish().len(), 6);
}

/// A program that asks for a hidden line, checks it survived intact, and asks again.
/// Each marker is split so that only running it, never echoing it, can print it.
fn secret_prompts() -> (&'static str, Value) {
    if cfg!(windows) {
        let script = "$first = Read-Host -Prompt ('NATIVE_' + 'PASSWORD_READY') -AsSecureString; $ptr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($first); try { $line = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($ptr); if (!$line.EndsWith([char]::ConvertFromUtf32(0x1F642))) { throw 'Unicode input lost' } } finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($ptr) }; Write-Output ('NATIVE_' + 'PTY_DONE'); $second = Read-Host -Prompt ('NATIVE_' + 'SECOND_READY') -AsSecureString; Write-Output ('NATIVE_' + 'PTY_END')";
        (
            "powershell.exe",
            strings(&["-NoLogo", "-NoProfile", "-Command", script]),
        )
    } else {
        let script = "stty -echo; printf 'NATIVE_''PASSWORD_READY\\n'; IFS= read -r first; stty echo; case \"$first\" in *🙂) ;; *) exit 9 ;; esac; printf 'NATIVE_''PTY_DONE\\n'; printf 'NATIVE_''SECOND_READY\\n'; IFS= read -r second; printf 'NATIVE_''PTY_END\\n'";
        ("/bin/sh", strings(&["-c", script]))
    }
}

pub(super) fn direct_input(path: &Path) {
    let (executable, args) = secret_prompts();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "native-pty",
                    "exec",
                    Value::object([
                        ("executable", Value::string(executable)),
                        ("args", args),
                        ("tty", Value::Bool(true)),
                        ("wait_ms", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (200, completion("Native terminal started.", vec![])),
    ]);
    let mut config = isolated_config(path);
    // Interactive terminals require explicit Full authority on this host.
    config.settings.default_permissions = Level::Full;
    config.store.save(&config.settings).unwrap();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(path).unwrap(),
    );
    agent
        .permissions()
        .set_default(config.settings.default_permissions);
    crate::tui::launch(agent, config, None, Default::default(), None).unwrap();
    let store = crate::sessions::Store::new(path.join("home"), path).unwrap();
    let sessions = store.list().unwrap().sessions;
    assert_eq!(sessions.len(), 1);
    let saved = store.fixture_load(&sessions[0].id).unwrap();
    let archive = format!("{:?}{:?}{:?}", saved.input, saved.messages, saved.events);
    assert!(!archive.contains("NATIVE_SECRET_42"));
    assert_eq!(saved.input.draft.text, "kept draft");
    assert_eq!(fixture.finish().len(), 2);
    println!("NATIVE_DIRECT_INPUT_RESTORED");
}

pub(super) fn background(path: &Path, scenario: &str) {
    if scenario == "background-light" {
        let terminal = Terminal::open().unwrap();
        let (width, height) = terminal.size;
        let frame = crate::tui::view::Frame {
            header: vec![],
            history: vec![],
            live: vec![
                crate::tui::line::Line::new("NATIVE_LIGHT_FRAME", crate::tui::theme::BODY),
                crate::tui::line::Line::default(),
            ],
            cursor: None,
            composer: 1,
        };
        let mut renderer = crate::tui::render::Renderer::new();
        renderer.set_light_background(true);
        write!(io::stdout(), "{}", renderer.paint(&frame, width, height)).unwrap();
        io::stdout().flush().unwrap();
        drop(terminal);
        println!("NATIVE_BACKGROUND_RESTORED");
    } else {
        // SGR 107 makes the main-screen palette entry white. The Windows adapter reads
        // that entry before the TUI enters its alternate screen; Unix terminals answer
        // the OSC 11 query instead, which their harness does.
        write!(io::stdout(), "\x1b[107;30m").unwrap();
        io::stdout().flush().unwrap();
        let fixture = HttpFixture::new(vec![]);
        let agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(path).unwrap(),
        );
        crate::tui::launch(agent, isolated_config(path), None, Default::default(), None).unwrap();
        write!(io::stdout(), "\x1b[0m").unwrap();
        io::stdout().flush().unwrap();
        assert!(fixture.finish().is_empty());
        println!("NATIVE_BACKGROUND_DETECTED_RESTORED");
    }
}

/// Saves conversations and instructions for the startup screen to list, then runs it.
fn startup_splash(store: Store) {
    let folder = std::env::current_dir().unwrap();
    let home = store.path().parent().unwrap().to_path_buf();
    std::fs::write(folder.join("AGENTS.md"), "# Fixture\n").unwrap();
    let skill = home.join("skills/review");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: review\ndescription: Inspect a change.\n---\n\n# Review\n",
    )
    .unwrap();
    let agent =
        |endpoint: String| Agent::new(OpenRouter::fixture(endpoint), Tools::new(&folder).unwrap());
    for title in [
        "Fix the session expiry check in auth",
        "Add the sleep test",
        "Migrate the config loader",
    ] {
        let mut saved = agent("http://127.0.0.1:1/chat/completions".into());
        saved.enable_sessions(&home).unwrap();
        let archive = saved.archive();
        for (role, text) in [("user", title), ("assistant", "Saved answer")] {
            archive.messages.lock().unwrap().push(Value::object([
                ("role", Value::string(role)),
                ("content", Value::string(text)),
            ]));
        }
        saved.save_session().unwrap();
    }
    let response = completion("Fixture answered.", vec![]);
    let fixture = HttpFixture::new(vec![(200, response)]);
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    crate::tui::launch(
        agent(fixture.endpoint.clone()),
        SessionConfig { store, settings },
        None,
        Default::default(),
        None,
    )
    .unwrap();
    assert_eq!(fixture.finish().len(), 1);
}

pub(super) fn startup(path: &Path, scenario: &str) {
    let store = Store::new(path.to_path_buf());
    if scenario == "startup-splash" {
        startup_splash(store);
        println!("NATIVE_STARTUP_RESTORED");
        return;
    }
    if scenario == "startup-fallback-new" {
        store
            .save(&Settings::new("isolated-fixture-key".into(), "fixture/fallback".into()).unwrap())
            .unwrap();
    }
    if !matches!(scenario, "startup-fresh" | "startup-unconfigured-prompt") {
        let mut settings =
            Settings::selected("openai", String::new(), "fixture/model".into()).unwrap();
        if matches!(scenario, "startup-fallback" | "startup-fallback-new") {
            settings
                .credentials
                .insert("openrouter".into(), "isolated-fixture-key".into());
        } else if matches!(scenario, "startup-key-removed" | "startup-setup-empty") {
            settings
                .select_backend("openrouter", String::new())
                .unwrap();
        }
        store.save(&settings).unwrap();
    }
    let saved = std::fs::read(store.path()).ok();
    if scenario == "startup-setup-empty" {
        crate::bootstrap::configure().unwrap();
    } else {
        crate::bootstrap::run(None, None, vec![], scenario == "startup-plain").unwrap();
    }
    if matches!(scenario, "startup-fresh" | "startup-unconfigured-prompt") {
        assert!(!store.path().exists());
    }
    assert_eq!(std::fs::read(store.path()).ok(), saved);
    if scenario == "startup-unconfigured-prompt" {
        let sessions = crate::sessions::Store::new(
            store.path().parent().unwrap().to_path_buf(),
            &std::env::current_dir().unwrap(),
        )
        .unwrap();
        assert!(sessions.list().unwrap().sessions.is_empty());
    }
    println!("NATIVE_STARTUP_RESTORED");
}
