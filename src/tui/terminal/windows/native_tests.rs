use super::*;
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    cancel::Cancellation,
    config::{Settings, Store},
    process,
    session::SessionConfig,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};

#[path = "activity_tests.rs"]
mod activity_tests;
#[path = "agents_tests.rs"]
mod agents_tests;
#[path = "work_tests.rs"]
mod work_tests;

#[path = "card_tests.rs"]
mod card_tests;

#[path = "native_direct_input.rs"]
mod native_direct_input;

#[path = "native_permissions.rs"]
mod native_permissions;

#[path = "native_background.rs"]
mod native_background;

#[path = "native_auth_panels.rs"]
mod native_auth_panels;

#[path = "startup_tests.rs"]
mod startup_tests;

fn native_pty_source() -> String {
    format!(
        "{}\n{}",
        include_str!("native_pty.cs"),
        include_str!("native_permissions.cs")
    )
    .replace(
        "JECODE_WARNING_FOREGROUND",
        &format!("\\x1b[{}m", crate::tui::theme::WARNING),
    )
    .replace(
        "JECODE_WARNING_SURFACE",
        &format!("\\x1b[{}m", crate::tui::theme::PILL_SURFACE),
    )
}

#[test]
#[ignore = "requires Windows ConPTY for an isolated native console check"]
fn native_windows_fullscreen_controls_and_terminal_lifetime() {
    for scenario in [
        "interactive",
        "delete-current",
        "input-recovery",
        "owner-death",
        "guardian-loss",
        "setup-cancel",
        "setup-success",
        "setup-error",
        "startup-removed",
        "startup-fresh",
        "startup-unconfigured-prompt",
        "startup-fallback",
        "startup-fallback-new",
        "startup-key-removed",
        "startup-plain",
        "startup-setup-empty",
        "auth-panels",
    ] {
        let directory = Directory::new();
        let script = format!(
            "Add-Type -TypeDefinition @'\n{}\n'@\n[JecodeTestPty]::Run('{}', '{}', '{}')\n",
            native_pty_source(),
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .replace('\'', "''"),
            directory.path().to_string_lossy().replace('\'', "''"),
            scenario
        );
        let result = process::run_observed(
            Command::new("powershell.exe")
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &script,
                ])
                .env("TEMP", directory.path())
                .env("TMP", directory.path())
                .env_remove("OPENROUTER_API_KEY"),
            None,
            Duration::from_secs(60),
            64 * 1024,
            &Cancellation::default(),
            None,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            result.exit_code,
            Some(0),
            "{scenario}: {}\n{}",
            String::from_utf8_lossy(&result.stderr),
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(!result.timed_out, "native {scenario} timed out");
    }
}

#[test]
#[ignore = "requires Windows ConPTY; explicit native input-to-VT latency measurement"]
fn native_windows_input_to_vt_latency() {
    for scenario in ["latency-idle", "latency-active"] {
        let directory = Directory::new();
        let script = format!(
            "Add-Type -TypeDefinition @'\n{}\n'@\n[JecodeTestPty]::Run('{}', '{}', '{}')\n",
            native_pty_source(),
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .replace('\'', "''"),
            directory.path().to_string_lossy().replace('\'', "''"),
            scenario,
        );
        let result = process::run_observed(
            Command::new("powershell.exe")
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &script,
                ])
                .env("TEMP", directory.path())
                .env("TMP", directory.path())
                .env_remove("OPENROUTER_API_KEY"),
            None,
            Duration::from_secs(120),
            64 * 1024,
            &Cancellation::default(),
            None,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            result.exit_code,
            Some(0),
            "{scenario}: {}\n{}",
            String::from_utf8_lossy(&result.stderr),
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(!result.timed_out, "native {scenario} timed out");
        println!("{}", String::from_utf8_lossy(&result.stdout));
    }
}

fn probe() -> String {
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n'@\n$bytes = [Text.Encoding]::UTF8.GetBytes([JecodeConsoleProbe]::Snapshot()); [Console]::OpenStandardOutput().Write($bytes, 0, $bytes.Length)",
        include_str!("native_probe.cs")
    );
    let result = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // Write UTF-8 to the pipe without changing the console's output code page.
    String::from_utf8(result.stdout).unwrap()
}

fn restored(path: &std::path::Path) {
    let before = std::fs::read_to_string(path.join("modes.txt")).unwrap();
    let after = probe();
    assert_eq!(
        after.lines().next().unwrap(),
        before,
        "native input/output modes changed"
    );
    assert!(
        after.contains("NATIVE_SHELL_SENTINEL"),
        "main screen was overwritten: {after}"
    );
    assert!(
        !after.contains("Native row"),
        "fullscreen transcript leaked into the shell: {after}"
    );
}

fn full_width_console() {
    let terminal = Terminal::open().unwrap();
    let (width, height) = terminal.size;
    let rows: Vec<_> = (0..height)
        .map(|row| {
            char::from(b'A' + (row % 26) as u8)
                .to_string()
                .repeat(width)
        })
        .collect();
    let frame = crate::tui::view::Frame {
        header: vec![],
        history: vec![],
        live: rows
            .iter()
            .map(|row| crate::tui::line::Line::new(row, crate::tui::theme::BODY))
            .collect(),
        cursor: Some((height - 1, width - 1)),
        composer: height - 1,
    };
    let output = crate::tui::render::Renderer::new().paint(&frame, width, height);
    write!(io::stdout(), "{output}").unwrap();
    io::stdout().flush().unwrap();
    let snapshot = probe();
    assert_eq!(snapshot.lines().nth(1).unwrap(), rows.concat());
    drop(terminal);
}

#[test]
#[ignore = "private child fixture of the native Windows ConPTY check"]
fn native_console_fixture() {
    let scenario = std::env::var("JECODE_NATIVE_CASE").unwrap();
    if scenario.starts_with("startup-") {
        startup_tests::fixture(&scenario);
        return;
    }
    if scenario.starts_with("setup-") {
        crate::setup::native_sign_in_fixture(&scenario);
        return;
    }
    let path = std::path::PathBuf::from(std::env::var_os("JECODE_NATIVE_DIR").unwrap());
    if scenario == "verify-owner-death" {
        restored(&path);
        println!("NATIVE_OWNER_RESTORED");
        return;
    }
    println!("NATIVE_SHELL_SENTINEL");
    std::fs::write(path.join("modes.txt"), probe().lines().next().unwrap()).unwrap();
    if scenario == "direct-input" {
        native_direct_input::fixture(&path);
        restored(&path);
        return;
    }
    if scenario == "permissions" {
        native_permissions::fixture(&path);
        restored(&path);
        println!("NATIVE_PERMISSIONS_RESTORED");
        return;
    }
    if scenario == "auth-panels" {
        native_auth_panels::console();
        restored(&path);
        println!("NATIVE_AUTH_PANELS_VERIFIED");
        return;
    }
    if matches!(scenario.as_str(), "background-light" | "background-native") {
        native_background::fixture(&path, &scenario);
        restored(&path);
        return;
    }
    if scenario == "latency-idle" || scenario == "latency-active" {
        super::measurement_tests::fixture(&path, scenario == "latency-active");
        restored(&path);
        println!("NATIVE_LATENCY_RESTORED");
        return;
    }
    if scenario == "owner-death" {
        let _terminal = Terminal::open().unwrap();
        println!("NATIVE_OWNER_READY");
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
    if scenario == "guardian-loss" {
        let mut terminal = Terminal::open().unwrap();
        terminal.close_for_fixture();
        assert!(terminal.check().is_err());
        drop(terminal);
        restored(&path);
        println!("NATIVE_GUARDIAN_RESTORED");
        return;
    }
    if scenario == "delete-current" {
        let fixture = HttpFixture::new(vec![(200, completion("Native completed.", vec![]))]);
        let settings =
            Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
        let config = SessionConfig {
            store: Store::new(path.join("home")),
            settings,
        };
        config.store.save(&config.settings).unwrap();
        let agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(&path).unwrap(),
        );
        crate::tui::launch(agent, config, None, Default::default(), None).unwrap();
        restored(&path);
        assert_eq!(fixture.finish().len(), 1);
        println!("NATIVE_DELETE_RESTORED");
        return;
    }
    if scenario == "input-recovery" {
        super::recovery_tests::fixture(&path);
        restored(&path);
        println!("NATIVE_INPUT_RECOVERY_RESTORED");
        return;
    }
    full_width_console();
    restored(&path);
    println!("NATIVE_FULL_WIDTH_VERIFIED");
    card_tests::console();
    restored(&path);
    println!("NATIVE_CARDS_VERIFIED");
    activity_tests::console();
    restored(&path);
    println!("NATIVE_ACTIVITY_VERIFIED");
    agents_tests::console();
    restored(&path);
    println!("NATIVE_AGENTS_VERIFIED");
    work_tests::console();
    restored(&path);
    println!("NATIVE_WORK_TOOLS_VERIFIED");
    std::fs::write(path.join("drop me.txt"), "dropped through ConPTY").unwrap();
    let response = (0..100)
        .map(|i| format!("Native row {i:03}\n"))
        .collect::<String>();
    let listing = crate::json::parse(
        r#"{"data":{"endpoints":[
        {"tag":"one/fp8","status":0,"supported_parameters":["tools"]},
        {"tag":"two/fp8","status":0,"supported_parameters":["tools"]}
        ]}}"#,
    )
    .unwrap();
    let content = (0..30)
        .map(|line| format!("native detail {line:02}\n"))
        .collect::<String>();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    tool_call(
                        "native-read",
                        "read",
                        crate::json::Value::object([
                            ("path", crate::json::Value::string("drop me.txt")),
                            (
                                "work",
                                crate::json::Value::string("Inspect the dropped file"),
                            ),
                        ]),
                    ),
                    tool_call(
                        "native-write",
                        "write",
                        crate::json::Value::object([
                            ("path", crate::json::Value::string("native-tool.txt")),
                            ("content", crate::json::Value::string(content)),
                        ]),
                    ),
                ],
            ),
        ),
        (200, completion(&response, vec![])),
        (200, listing),
    ]);
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let config = SessionConfig {
        store: Store::new(path.join("home")),
        settings,
    };
    config.store.save(&config.settings).unwrap();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&path).unwrap(),
    );
    crate::tui::launch(agent, config, None, Default::default(), None).unwrap();
    restored(&path);
    let sessions = crate::sessions::Store::new(path.join("home"), &path)
        .unwrap()
        .list()
        .unwrap()
        .sessions;
    assert_eq!(sessions.len(), 1);
    let saved = crate::sessions::Store::new(path.join("home"), &path)
        .unwrap()
        .fixture_load(&sessions[0].id)
        .unwrap();
    assert_eq!(
        saved.input.draft.text,
        format!("kept draft β🙂{} ", crate::attachments::MARKER)
    );
    assert_eq!(saved.input.draft.attachments[0].name, "drop me.txt");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].headers[0].starts_with("GET /models/fixture/model/endpoints "));
    println!("NATIVE_RESTORED");
}
