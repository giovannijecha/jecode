mod driver;
use super::*;
use crate::{
    agent::Agent,
    config::{Settings, Store},
    json::Value,
    openrouter::OpenRouter,
    session::SessionConfig,
    test_support::{HttpFixture, completion, tool_call},
    tools::Tools,
};

#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires util-linux script to exercise an isolated native Linux PTY"]
fn native_linux_tui_controls_and_terminal_lifetime() {
    controls_and_terminal_lifetime();
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "requires the system script tool to exercise an isolated native macOS PTY"]
fn native_macos_tui_controls_and_terminal_lifetime() {
    controls_and_terminal_lifetime();
}

fn controls_and_terminal_lifetime() {
    let mut terminal = driver::Pty::new("interactive");
    terminal.wait("Ask anything");
    terminal.send(b"\x03/help\r");
    terminal.wait("Commands and controls");
    terminal.send(b"/settings\r");
    let settings = terminal.wait("Settings · saved defaults");
    terminal.send(b"3");
    let key = terminal.wait_after("Enter validate and save", settings);
    terminal.send(b"\x03");
    terminal.wait_after("Settings · saved defaults", key);
    terminal.send(b"\x03");
    terminal.resize(40, 12);
    terminal.resize(5, 2);
    terminal.resize(80, 24);
    terminal.send("\x1b[200~first α🙂\nline\x1b[201~".as_bytes());
    thread::sleep(Duration::from_millis(200));
    assert!(!terminal.directory().join("command-started").exists());
    terminal.send(b"\r");
    terminal.wait_file("command-started");
    terminal.send(b"queued follow-up\rkept draft\x03");
    terminal.wait("Interrupted");
    terminal.send(b"\x03second prompt\r");
    terminal.wait("Native second answer");
    terminal.send(b"\x1b[<64;10;5M");
    terminal.wait("Back to bottom");
    terminal.send(b"\x1b[5~\x1b[6~\x1b[A\x1b[B");
    terminal.send("\x1b[200~unsent β🙂\nkept\x1b[201~".as_bytes());
    terminal.send(b"\x11");
    let resume = terminal.wait("PTY_PHASE_RESUME");
    terminal.wait_after("unsent β", resume);
    terminal.send(b"\x03\x11");
    let manage = terminal.wait("PTY_PHASE_MANAGE");
    terminal.wait_after("Ctrl+D delete", manage);
    terminal.send(b"\x04");
    let armed = terminal.wait_after("Delete?", manage);
    terminal.send(b"\x03");
    let cancelled = terminal.wait_after("Ctrl+D delete", armed);
    terminal.send(b"\x04\r");
    let deleted = terminal.wait_after("Deleted Other saved conversation", cancelled);
    terminal.wait_after("Ctrl+D delete", deleted);
    terminal.send(b"\r");
    let resumed = terminal.wait_after("Native second answer", deleted);
    terminal.send(b"/resume\r\x04\r");
    let current = terminal.wait_after("Conversation deleted", resumed);
    terminal.wait_after("No saved conversations", current);
    terminal.send(b"\x11");
    terminal.finish("PTY_VERIFIED");

    let mut terminal = driver::Pty::new("owner-death");
    terminal.wait("PTY_OWNER_READY");
    terminal.kill_owner();
    terminal.finish("PTY_OWNER_RESTORED");

    let mut terminal = driver::Pty::new("guardian-loss");
    terminal.finish("PTY_GUARDIAN_RECOVERED");
}

#[test]
#[ignore = "private child fixture of the native Unix PTY check"]
fn native_pty_fixture() {
    let path = std::path::PathBuf::from(std::env::var_os("JECODE_TUI_FIXTURE").unwrap());
    let output = std::process::Command::new("tty")
        .stdin(std::process::Stdio::inherit())
        .output()
        .unwrap();
    assert!(output.status.success(), "native fixture has no terminal");
    let tty = std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    eprintln!("\nPTY_META|{}|{}", std::process::id(), tty.display());
    let tty = File::open(tty).unwrap();
    let original = mode::stty(&tty, &["-g"]).unwrap();
    let bash = crate::tools::find_bash().unwrap();
    if std::env::var("JECODE_TUI_CASE").unwrap() == "guardian-loss" {
        let mut terminal = Terminal::open(&bash).unwrap();
        terminal.mode.close_for_fixture();
        assert!(terminal.check().is_err());
        let at = Instant::now();
        drop(terminal);
        assert!(at.elapsed() < Duration::from_secs(1));
        assert_eq!(mode::stty(&tty, &["-g"]).unwrap(), original);
        eprintln!("PTY_GUARDIAN_RECOVERED");
        return;
    }
    if std::env::var("JECODE_TUI_CASE").unwrap() == "owner-death" {
        let _mode = mode::Mode::open(tty, &bash).unwrap();
        eprintln!("PTY_OWNER_READY");
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
    let fixture = HttpFixture::with_wait(
        vec![
            (
                200,
                completion(
                    "Running a native foreground command.",
                    vec![tool_call(
                        "native-command",
                        "bash",
                        Value::object([(
                            "command",
                            Value::string(
                                "printf started > command-started; sleep 30; printf late > late.txt",
                            ),
                        )]),
                    )],
                ),
            ),
            (
                200,
                completion(
                    &format!(
                        "# Native second answer\n{}Native second answer\nReady after interruption.",
                        "Retained native conversation row.\n".repeat(40)
                    ),
                    vec![],
                ),
            ),
        ],
        Duration::from_secs(20),
    );
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let config = || SessionConfig {
        store: Store::new(path.join("home")),
        settings: settings.clone(),
        bash: bash.clone(),
    };
    config().store.save(&settings).unwrap();
    let agent = || {
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(&path).unwrap(),
        )
    };
    crate::tui::run(agent(), config(), Default::default()).unwrap();
    assert_eq!(mode::stty(&tty, &["-g"]).unwrap(), original);
    assert!(!path.join("late.txt").exists());
    let sessions = crate::sessions::Store::new(path.join("home"), &path).unwrap();
    let listing = sessions.list().unwrap();
    assert_eq!(listing.sessions.len(), 1);
    let id = &listing.sessions[0].id;
    let saved = sessions.fixture_load(id).unwrap();
    assert_eq!(saved.input.draft.text, "unsent β🙂\nkept");
    assert!(!saved.pending.active);
    let prompts: Vec<_> = saved
        .messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .map(|message| message.get("content").unwrap().as_str().unwrap())
        .collect();
    assert_eq!(prompts, ["first α🙂\nline", "second prompt"]);
    assert!(
        saved
            .input
            .history
            .iter()
            .any(|prompt| prompt == "queued follow-up")
    );
    assert!(
        saved
            .events
            .iter()
            .any(|event| event.get("type").and_then(Value::as_str) == Some("turn_error"))
    );
    eprintln!("\r\nPTY_PHASE_RESUME");
    crate::tui::resume(agent(), config(), Some(id.clone())).unwrap();
    assert_eq!(mode::stty(&tty, &["-g"]).unwrap(), original);
    assert!(
        sessions
            .fixture_load(id)
            .unwrap()
            .input
            .draft
            .text
            .is_empty()
    );
    let mut other = agent();
    other.enable_sessions(&path.join("home")).unwrap();
    other
        .archive()
        .messages
        .lock()
        .unwrap()
        .push(Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Other saved conversation")),
        ]));
    other.save_session().unwrap();
    drop(other);
    eprintln!("\r\nPTY_PHASE_MANAGE");
    crate::tui::resume(agent(), config(), None).unwrap();
    assert_eq!(mode::stty(&tty, &["-g"]).unwrap(), original);
    assert!(sessions.list().unwrap().sessions.is_empty());
    assert!(sessions.fixture_load(id).is_err());
    assert_eq!(fixture.finish().len(), 2);
    eprintln!("PTY_VERIFIED");
}
