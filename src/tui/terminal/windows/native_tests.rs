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

#[test]
#[ignore = "requires Windows ConPTY for an isolated native console check"]
fn native_windows_fullscreen_controls_and_terminal_lifetime() {
    for scenario in [
        "interactive",
        "delete-current",
        "owner-death",
        "guardian-loss",
    ] {
        let directory = Directory::new();
        let script = format!(
            "Add-Type -TypeDefinition @'\n{}\n'@\n[JecodeTestPty]::Run('{}', '{}', '{}')\n",
            include_str!("native_pty.cs"),
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
            include_str!("native_pty.cs"),
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
        "Add-Type -TypeDefinition @'\n{}\n'@\n[JecodeConsoleProbe]::Snapshot()",
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

fn full_width_console(bash: &std::path::Path) {
    let terminal = Terminal::open(bash).unwrap();
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

fn tool_cards_console(bash: &std::path::Path) {
    use crate::{events::Event, json::Value, tui::state::State};
    let terminal = Terminal::open(bash).unwrap();
    let (width, height) = terminal.size;
    let mut state = State {
        width,
        height,
        ..State::default()
    };
    state.event(Event::ToolStarted {
        id: "update".into(),
        name: "edit".into(),
        arguments: Value::object([("path", Value::string("src/limit.rs"))]),
    });
    state.event(Event::ToolChanged {
        id: "update".into(),
        change: crate::file_change::Change::between(
            Some("let limit = 10;\n"),
            "let limit = LOGIN_LIMIT;\n",
        )
        .unwrap(),
    });
    state.event(Event::ToolFinished {
        id: "update".into(),
        name: "edit".into(),
        summary: "wrote file".into(),
        result: Value::object([("bytes_written", Value::number(25))]),
    });
    state.event(Event::ToolStarted {
        id: "verify".into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string("cargo test --locked --offline"))]),
    });
    let result = Value::object([
        ("exit_code", Value::number(0)),
        (
            "checkpoint",
            Value::object([("status", Value::string("partial"))]),
        ),
    ]);
    state.event(Event::ToolFinished {
        id: "verify".into(),
        name: "bash".into(),
        summary: crate::events::tool_summary("bash", &result),
        result,
    });
    for (id, name, arguments, result) in [
        (
            "fetch",
            "web",
            Value::object([
                ("action", Value::string("fetch")),
                ("url", Value::string("https://example.org/")),
            ]),
            Value::object([
                ("status", Value::number(200)),
                ("bytes", Value::number(12)),
                ("path", Value::string("tmp:web/fetches/page.body")),
            ]),
        ),
        (
            "capture",
            "browser",
            Value::object([
                ("action", Value::string("screenshot")),
                ("tab_id", Value::string("tab-1")),
            ]),
            Value::object([
                ("tab_id", Value::string("tab-1")),
                (
                    "screenshot_path",
                    Value::string("tmp:browser/screenshot.png"),
                ),
            ]),
        ),
    ] {
        state.event(Event::ToolStarted {
            id: id.into(),
            name: name.into(),
            arguments,
        });
        state.event(Event::ToolFinished {
            id: id.into(),
            name: name.into(),
            summary: crate::events::tool_summary(name, &result),
            result,
        });
    }
    let rows: Vec<_> = state.rows().into_iter().flatten().collect();
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("Checkpoint partial")),
        "partial checkpoint status is missing from the Bash tool card"
    );
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("1 −  let limit = 10;"))
    );
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("1 +  let limit = LOGIN_LIMIT;"))
    );
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("Web(fetch https://example.org/)"))
    );
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("Browser(screenshot tab-1)"))
    );
    assert!(rows.iter().any(|row| {
        row.plain()
            .contains("screenshot saved to tmp:browser/screenshot.png")
    }));
    let expected: String = (0..height)
        .map(|index| {
            let text = rows.get(index).map_or_else(String::new, |row| row.plain());
            format!(
                "{text}{}",
                " ".repeat(width.saturating_sub(crate::tui::text::cells(&text)))
            )
        })
        .collect();
    let frame = crate::tui::view::Frame {
        header: vec![],
        history: vec![],
        live: rows,
        cursor: None,
        composer: height - 1,
    };
    let output = crate::tui::render::Renderer::new().paint(&frame, width, height);
    write!(io::stdout(), "{output}").unwrap();
    io::stdout().flush().unwrap();
    let snapshot = probe();
    assert_eq!(snapshot.lines().nth(1).unwrap(), expected);
    drop(terminal);
}

#[test]
#[ignore = "private child fixture of the native Windows ConPTY check"]
fn native_console_fixture() {
    let path = std::path::PathBuf::from(std::env::var_os("JECODE_NATIVE_DIR").unwrap());
    let scenario = std::env::var("JECODE_NATIVE_CASE").unwrap();
    if scenario == "verify-owner-death" {
        restored(&path);
        println!("NATIVE_OWNER_RESTORED");
        return;
    }
    println!("NATIVE_SHELL_SENTINEL");
    std::fs::write(path.join("modes.txt"), probe().lines().next().unwrap()).unwrap();
    let bash = crate::tools::find_bash().unwrap();
    if scenario == "latency-idle" || scenario == "latency-active" {
        super::measurement_tests::fixture(&path, &bash, scenario == "latency-active");
        restored(&path);
        println!("NATIVE_LATENCY_RESTORED");
        return;
    }
    if scenario == "owner-death" {
        let _terminal = Terminal::open(&bash).unwrap();
        println!("NATIVE_OWNER_READY");
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
    if scenario == "guardian-loss" {
        let mut terminal = Terminal::open(&bash).unwrap();
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
            bash,
        };
        config.store.save(&config.settings).unwrap();
        let agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(&path).unwrap(),
        );
        crate::tui::run(agent, config, Default::default()).unwrap();
        restored(&path);
        assert_eq!(fixture.finish().len(), 1);
        println!("NATIVE_DELETE_RESTORED");
        return;
    }
    full_width_console(&bash);
    restored(&path);
    tool_cards_console(&bash);
    restored(&path);
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
                        crate::json::Value::object([(
                            "path",
                            crate::json::Value::string("drop me.txt"),
                        )]),
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
        bash,
    };
    config.store.save(&config.settings).unwrap();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&path).unwrap(),
    );
    crate::tui::run(agent, config, Default::default()).unwrap();
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
