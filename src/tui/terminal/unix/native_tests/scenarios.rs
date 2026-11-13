//! Unix runs of the conversations in `terminal::fixtures`, also driven on Windows by
//! `native_pty.cs`, plus the colour and keyboard paths that only Unix terminals take.

use super::driver::Pty;
use super::*;
use crate::tui::terminal::fixtures;
use std::path::Path;

fn pause(milliseconds: u64) {
    thread::sleep(Duration::from_millis(milliseconds));
}

fn command(terminal: &mut Pty, text: &str) {
    terminal.send(text.as_bytes());
    pause(120);
    terminal.send(b"\r");
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|bytes| bytes == needle)
}

/// The colour as it appears in an SGR sequence, which may also set bold or a fill.
fn escape_for(style: impl std::fmt::Display) -> Vec<u8> {
    format!("{style}m").into_bytes()
}

/// Runs a conversation in the fixture process. False for the cases handled elsewhere.
pub(super) fn fixture(case: &str, path: &Path, tty: &File, saved: &str) -> bool {
    match case {
        "permissions" => {
            fixtures::permissions(path);
            assert_mode_restored(tty, saved);
            println!("NATIVE_PERMISSIONS_RESTORED");
        }
        "direct-input" => {
            fixtures::direct_input(path);
            assert_mode_restored(tty, saved);
        }
        "background-light" | "background-native" => {
            fixtures::background(path, case);
            assert_mode_restored(tty, saved);
        }
        "palette" => {
            palette_conversation(path);
            assert_mode_restored(tty, saved);
            println!("NATIVE_PALETTE_RESTORED");
        }
        "terminal-probe" => {
            terminal_probe_conversation(path);
            assert_mode_restored(tty, saved);
            println!("NATIVE_TERMINAL_PROBE_RESTORED");
        }
        "latency" => {
            idle_conversation(path);
            assert_mode_restored(tty, saved);
            println!("NATIVE_LATENCY_RESTORED");
        }
        _ if case.starts_with("startup-") => {
            fixtures::startup(path, case);
            assert_mode_restored(tty, saved);
        }
        _ => return false,
    }
    true
}

fn launch(path: &Path, fixture: &HttpFixture) {
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let config = SessionConfig {
        store: Store::new(path.join("home")),
        settings,
    };
    config.store.save(&config.settings).unwrap();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(path).unwrap(),
    );
    crate::tui::launch(
        agent,
        config,
        None,
        Default::default(),
        None,
        Default::default(),
    )
    .unwrap();
}

fn idle_conversation(path: &Path) {
    let fixture = HttpFixture::new(vec![]);
    launch(path, &fixture);
    assert!(fixture.finish().is_empty());
}

/// A heading, body text and a diff card, the roles the palette maps most differently.
fn palette_conversation(path: &Path) {
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "paint",
                    "write",
                    Value::object([
                        ("path", Value::string("painted.txt")),
                        ("content", Value::string("added line\n")),
                    ]),
                )],
            ),
        ),
        (
            200,
            completion("# Palette heading\nPalette answer.", vec![]),
        ),
    ]);
    launch(path, &fixture);
    assert_eq!(
        std::fs::read_to_string(path.join("painted.txt")).unwrap(),
        "added line\n"
    );
    let store = crate::sessions::Store::new(path.join("home"), path).unwrap();
    let sessions = store.list().unwrap().sessions;
    let saved = store.fixture_load(&sessions[0].id).unwrap();
    assert_eq!(saved.input.draft.text, "first\nsecond");
    assert_eq!(fixture.finish().len(), 2);
}

/// Workspace's default level, under a process that does have a controlling terminal.
fn terminal_probe_conversation(path: &Path) {
    let probe =
        "if (: < /dev/tty) 2> /dev/null; then echo reachable; else echo detached; fi > tty-probe";
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "probe",
                    "exec",
                    Value::object([
                        ("executable", Value::string("/bin/sh")),
                        (
                            "args",
                            Value::Array(vec![Value::string("-c"), Value::string(probe)]),
                        ),
                    ]),
                )],
            ),
        ),
        (200, completion("Terminal probed.", vec![])),
    ]);
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let config = SessionConfig {
        store: Store::new(path.join("home")),
        settings,
    };
    config.store.save(&config.settings).unwrap();
    // Workspace refuses a working folder that holds the Jecode home.
    let project = path.join("project");
    std::fs::create_dir(&project).unwrap();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&project).unwrap(),
    );
    agent
        .permissions()
        .set_default(crate::permissions::Level::Workspace);
    crate::tui::launch(
        agent,
        config,
        None,
        Default::default(),
        None,
        Default::default(),
    )
    .unwrap();
    let requests = fixture.finish();
    assert_eq!(
        std::fs::read_to_string(project.join("tty-probe"))
            .ok()
            .as_deref(),
        Some("detached\n"),
        "{:?}",
        requests.last().map(|request| &request.body)
    );
}

/// With the terminal, a command could type `/permissions full` into Jecode (TIOCSTI).
#[test]
#[ignore = "requires the system script tool for a native Unix PTY"]
fn native_unix_confined_commands_have_no_terminal() {
    let mut t = Pty::new("terminal-probe");
    t.wait("Ask anything");
    command(&mut t, "probe");
    t.wait("Terminal probed.");
    t.quit_with(b"\x11", "NATIVE_TERMINAL_PROBE_RESTORED");
}

#[test]
#[ignore = "requires the system script tool for a native Unix PTY"]
fn native_unix_permissions() {
    let mut t = Pty::new("permissions");
    t.wait("Ask anything");
    command(&mut t, "/permissions");
    t.wait("Permissions");
    t.wait("this conversation");
    let narrowed = t.mark();
    t.resize(44, 12);
    t.wait_after("Permissions", narrowed);
    t.send(b"1");
    t.wait("Read");
    t.resize(80, 24);

    command(&mut t, "/settings");
    t.wait("Default permissions");
    let defaults = t.mark();
    t.send(b"4");
    t.wait_after("Read and analyze; no project changes", defaults);
    let saved = t.mark();
    t.send(b"3");
    t.wait_after("this conversation stays Read", saved);
    t.send(b"\x1b");
    t.wait_screen("Ask anything", "Settings");

    let level = t.mark();
    command(&mut t, "/permissions");
    t.wait_after("new conversations use Full", level);
    let applied = t.mark();
    t.send(b"2");
    t.wait_after("this conversation", applied);
    command(&mut t, "read outside");
    t.wait("Waiting for permission");
    t.wait("Read access outside this folder");
    t.wait("Approve and remember");
    t.send(b"2");
    t.wait("Native first read complete.");

    let remembered = t.mark();
    command(&mut t, "/permissions");
    t.wait_after("1 for this conversation", remembered);
    let grants = t.mark();
    t.send(b"4");
    t.wait_after("outside-note.txt", grants);
    t.send(b"\x04");
    t.wait("Nothing remembered");
    t.send(b"\x1b");
    t.wait_screen("Permissions", "Permissions › Remembered");
    t.send(b"\x1b");
    t.wait_screen("Ask anything", "Permissions");

    let request = t.mark();
    command(&mut t, "read outside again");
    t.wait_after("Waiting for permission", request);
    t.send(b"3");
    t.wait("Native second read complete.");

    // A call too long for the dock pages to its end; the choices still decide.
    let paged = t.mark();
    command(&mut t, "run the long command");
    t.wait_after("PgUp/PgDn", paged);
    t.wait_after("native-step-01", paged);
    for _ in 0..5 {
        t.send(b"\x1b[6~");
        pause(120);
    }
    t.wait_after("native-step-40", paged);
    t.send(b"3");
    t.wait_after("nothing ran; the agent was told", paged);
    t.wait_after("Native long command denied.", paged);
    t.send(b"kept draft");
    t.wait("kept draft");
    t.quit_with(b"\x11", "NATIVE_PERMISSIONS_RESTORED");
}

#[test]
#[ignore = "requires the system script tool for a native Unix PTY"]
fn native_unix_direct_input() {
    let mut t = Pty::new("direct-input");
    t.wait("Ask anything");
    command(&mut t, "start terminal");
    t.wait("Native terminal started.");
    t.wait("  PROC   1 running");
    command(&mut t, "/process");
    t.wait("1 running");
    t.send(b"\r");
    t.wait("i input");
    let started = t.mark();
    t.send(b"i");
    t.wait_after("Ctrl+] back", started);
    t.wait("NATIVE_PASSWORD_READY");
    t.send("NATIVE_SECRET_42🙂\r".as_bytes());
    t.wait_after("NATIVE_PTY_DONE", started);
    t.ensure_no_text("NATIVE_SECRET_42", started);
    let detached = t.mark();
    t.send(b"\x1d");
    t.wait_after("i input", detached);
    // Match the new input-mode frame, not an earlier queued repaint.
    let reattached = t.mark();
    t.send(b"i");
    t.wait_after("Ctrl+] back", reattached);
    // The keyboard protocol reports the same key differently, and it must still leave.
    let reported = t.mark();
    t.send(b"\x1b[93;5u");
    t.wait_after("i input", reported);
    let again = t.mark();
    t.send(b"i");
    t.wait_after("Ctrl+] back", again);
    t.wait("NATIVE_SECOND_READY");
    t.send(b"finish\r");
    t.wait_after("NATIVE_PTY_END", detached);
    t.wait_for_terminal_exit(detached);
    // Two Escapes inside the parser's wait for a sequence would count as one.
    t.send(b"\x1b");
    pause(200);
    t.send(b"\x1b");
    pause(200);
    // The process ended by itself, so the idle conversation continued.
    t.wait_after("Native terminal finished.", detached);
    t.send(b"kept draft");
    t.wait("kept draft");
    t.quit_with(b"\x11", "NATIVE_DIRECT_INPUT_RESTORED");
}

/// What the light palette draws, as the bytes a terminal receives.
fn light_escape(style: impl std::fmt::Display) -> Vec<u8> {
    crate::tui::theme::set_light(true);
    let escape = escape_for(style);
    crate::tui::theme::set_light(false);
    escape
}

/// The live opening paints only the caret, level chip and AGENTS badge; every
/// other cell must retain the terminal background. Check all complete SGR fills,
/// as well as the resulting screen, rather than banning those intentional fills.
fn opening_backgrounds(raw: &[u8], painted: usize) {
    use crate::tui::theme::{BADGE_EDIT, BADGE_IDLE, CURSOR};

    let background = |escape: Vec<u8>| {
        let start = escape
            .windows(5)
            .position(|bytes| bytes == b"48;2;")
            .expect("explicit native background");
        escape[start..].to_vec()
    };
    let allowed = [
        background(escape_for(CURSOR)),
        background(light_escape(CURSOR)),
        escape_for(BADGE_IDLE),
        light_escape(BADGE_IDLE),
        escape_for(BADGE_EDIT),
        light_escape(BADGE_EDIT),
    ];
    let mut position = 0;
    let mut fills = 0;
    while let Some(offset) = raw[position..]
        .windows(5)
        .position(|bytes| bytes == b"48;2;")
    {
        let start = position + offset;
        let Some(end) = raw[start..].iter().position(|byte| *byte == b'm') else {
            break; // A read may end inside the last, incomplete SGR sequence.
        };
        let sequence = &raw[start..=start + end];
        assert!(
            allowed.iter().any(|role| role == sequence),
            "unexpected background escape: {}",
            String::from_utf8_lossy(sequence)
        );
        fills += 1;
        position = start + end + 1;
    }
    assert!(fills > 0, "the opening did not paint its caret or chips");
    assert!(
        contains(raw, b";49m"),
        "default background was never restored"
    );
    let max_painted = " Workspace ".len() + " AGENTS ".len() + 1; // one caret
    assert!(painted > 0, "no complete opening frame was inspected");
    assert!(
        painted <= max_painted,
        "opening painted {painted} cells, beyond its caret and chips"
    );
}

#[test]
#[ignore = "requires the system script tool for a native Unix PTY"]
fn native_unix_light_background() {
    let mut t = Pty::new("background-light");
    t.wait_raw(&light_escape(crate::tui::theme::BODY), 0);
    t.wait("NATIVE_LIGHT_FRAME");
    assert!(!contains(t.raw(), b"48;2;"));
    t.finish("NATIVE_BACKGROUND_RESTORED");

    // The terminal's own answer decides: a light one brings the light inks, on nothing.
    let placeholder = light_escape(crate::tui::theme::DIM);
    for (reply, light) in [
        (&b"\x1b]11;rgb:ffff/ffff/ffff\x07"[..], true),
        (&b"\x1b]11;rgb:0000/0000/0000\x1b\\"[..], false),
    ] {
        let mut t = Pty::new("background-native");
        t.wait_raw(b"\x1b]11;?\x07", 0);
        t.send(reply);
        t.wait("Ask anything");
        pause(300);
        t.wait_for_output(0);
        t.drain();
        if light {
            t.wait_raw(&placeholder, 0);
        } else {
            assert!(!contains(t.raw(), &placeholder));
        }
        opening_backgrounds(t.raw(), t.painted_cells());
        t.send(b"\x11");
        t.finish("NATIVE_BACKGROUND_DETECTED_RESTORED");
    }
}

/// Bytes this terminal has never been asked to understand must never reach it, and
/// keys reported through the keyboard protocol must act like plain ones.
#[test]
#[ignore = "requires the system script tool for a native Unix PTY"]
fn native_unix_palette_and_keyboard_protocol() {
    let mut t = Pty::new("palette");
    t.wait("Ask anything");
    command(&mut t, "paint");
    let answer = t.wait("Palette answer.");
    t.wait_after("Done", answer);
    t.send(b"throwaway");
    let typed = t.wait("throwaway");
    t.send(b"\x1b[99;5u");
    t.wait_after("Ask anything", typed);
    t.send(b"first\x1b[13;2usecond");
    t.wait("second");
    t.quit_with(b"\x1b[113;5u", "NATIVE_PALETTE_RESTORED");
    let raw = t.raw();
    assert!(
        contains(raw, b"\x1b[>5u"),
        "keyboard protocol not requested"
    );
    assert!(contains(raw, b"\x1b[<u"), "keyboard protocol not released");
    assert!(contains(raw, b"38;5;"), "palette colours were not used");
    assert!(
        !contains(raw, b"38;2;"),
        "24-bit foreground reached the terminal"
    );
    assert!(
        !contains(raw, b"48;2;"),
        "24-bit background reached the terminal"
    );
}

fn startup_warnings(t: &mut Pty, scenario: &str) {
    t.wait("Ask anything");
    t.wait(if scenario.starts_with("startup-fallback") {
        "Default provider unavailable"
    } else {
        "No configured provider"
    });
    t.wait_raw(&escape_for(crate::tui::theme::WARNING), 0);
    if scenario == "startup-unconfigured-prompt" {
        let typed = t.mark();
        t.send(b"Keep my task");
        t.wait_after("Keep my task", typed);
        let request = t.mark();
        t.send(b"\r");
        t.wait_after("No configured provider", request);
        let cleared = t.mark();
        t.send(b"\x15");
        t.wait_after("Ask anything", cleared);
    }
    let next = t.mark();
    t.send(b"/new");
    t.wait_after("Start a new conversation", next);
    t.send(b"\r");
    t.wait_after(
        match scenario {
            "startup-fallback-new" => "current model kept",
            "startup-fallback" => "Choose a model",
            _ => "No configured provider",
        },
        next,
    );
    t.ensure_no_text("has no configured account", next);
    let settings = t.mark();
    t.send(b"/settings");
    t.wait_after("Change saved defaults", settings);
    t.send(b"\r");
    t.wait_after("Providers", settings);
    t.send(b"\x1b");
    pause(200);
    t.send(b"\x11");
}

#[test]
#[ignore = "requires the system script tool for a native Unix PTY"]
fn native_unix_startup() {
    for scenario in [
        "startup-removed",
        "startup-fresh",
        "startup-unconfigured-prompt",
        "startup-fallback",
        "startup-fallback-new",
        "startup-key-removed",
    ] {
        let mut t = Pty::new(scenario);
        startup_warnings(&mut t, scenario);
        t.finish("NATIVE_STARTUP_RESTORED");
    }

    let mut t = Pty::new("startup-plain");
    t.wait("Providers:");
    t.send(b"/cancel\r");
    t.finish("NATIVE_STARTUP_RESTORED");

    let mut t = Pty::new("startup-setup-empty");
    t.wait("Providers:");
    t.send(b"1\r");
    t.wait("OpenRouter API key (hidden):");
    t.ensure_no_text("Enter keeps the saved key", 0);
    t.send(b"/cancel\r");
    t.finish("NATIVE_STARTUP_RESTORED");
}

#[test]
#[ignore = "requires the system script tool for a native Unix PTY"]
fn native_unix_startup_splash() {
    let mut t = Pty::new("startup-splash");
    t.wait("Ask anything");
    let full = t.mark();
    t.resize(100, 32);
    let image_key = if cfg!(target_os = "macos") {
        "Ctrl+V"
    } else {
        "Alt+V"
    };
    t.wait_after(
        &format!("/ commands   {image_key} image   Ctrl+Q quit"),
        full,
    );
    t.wait_after("✓ 1 skill   ✓ AGENTS.md", full);
    t.wait_after("Recent in this folder", full);
    t.wait_after("Fix the session expiry check in auth", full);
    t.wait_after("/resume all", full);
    t.wait_raw(&escape_for(crate::tui::theme::ACCENT), 0);
    t.wait_raw(&escape_for(crate::tui::theme::LIT), 0);

    let medium = t.mark();
    t.resize(64, 16);
    t.wait_after("▄", medium);
    t.wait_after("Recent in this folder", medium);

    let narrow = t.mark();
    t.resize(48, 16);
    t.wait_after(">_ jecode", narrow);
    t.wait_after("Recent in this folder", narrow);

    let restored = t.mark();
    t.resize(100, 32);
    t.wait_after("/ commands", restored);

    // The command key arrives in the same input burst as the still unsent draft.
    let commands = t.mark();
    t.send(b"burst draft\x07");
    t.wait_after("draft kept", commands);
    t.wait_after(
        &format!("{} commands", crate::session::commands::COMMANDS.len()),
        commands,
    );
    let filtered = t.mark();
    t.send(b"mod");
    t.wait_after(
        &format!("1 of {} commands", crate::session::commands::COMMANDS.len()),
        filtered,
    );
    let returned = t.mark();
    t.send(b"\x1b");
    t.wait_after("Ctrl+G commands", returned);
    t.wait_after("burst draft", returned);
    t.send(b"\x15");
    pause(200);

    let prompted = t.mark();
    t.send(b"first prompt");
    t.wait_after("first prompt", prompted);
    t.send(b"\r");
    t.wait_after("Fixture answered.", prompted);
    t.wait_after("Done", prompted);
    t.wait_after(">_ jecode", prompted);
    t.quit_with(b"\x11", "NATIVE_STARTUP_RESTORED");
}

/// Not part of the check: prints how long a typed key takes to reach the terminal.
#[test]
#[ignore = "explicit native input-to-VT measurement on a Unix PTY"]
fn measure_unix_input_to_vt_latency() {
    let mut t = Pty::new("latency");
    t.wait("Ask anything");
    pause(500);
    let mut samples = Vec::new();
    for index in 0..200 {
        // A repaint may arrive in pieces; only what follows this key counts.
        pause(30);
        t.drain();
        let seen = t.raw().len();
        let started = Instant::now();
        t.send(&[b'a' + (index % 26) as u8]);
        t.wait_for_output(seen);
        samples.push(started.elapsed());
    }
    samples.sort();
    println!(
        "input to VT, {} keys: median {:?}, p95 {:?}, max {:?}",
        samples.len(),
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100],
        samples[samples.len() - 1]
    );
    t.send(b"\x15\x11");
    t.finish("NATIVE_LATENCY_RESTORED");
}
