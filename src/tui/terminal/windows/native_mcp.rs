//! B14 panels painted through the actual Windows ConPTY and terminal renderer.
use super::{Terminal, probe};
use crate::{
    backend::Routing,
    effort::Effort,
    events::Event,
    json::Value,
    tui::{
        Kind, information::Information, mcp::Panel, render::Renderer, selector::Selector,
        state::State, view,
    },
};
use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires Windows ConPTY and PowerShell for native MCP panels"]
fn native_windows_mcp_panels() {
    let directory = crate::test_support::Directory::new();
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n'@\n[JecodeTestPty]::Run('{}', '{}', 'mcp-panels')\n",
        super::native_pty_source(),
        std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .replace('\'', "''"),
        directory.path().to_string_lossy().replace('\'', "''"),
    );
    let result = crate::process::run_observed(
        std::process::Command::new("powershell.exe")
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
        &crate::cancel::Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(!result.timed_out);
}

pub(super) fn console() {
    let terminal = Terminal::open().unwrap();
    for columns in [44, 100] {
        wait_for_width(&terminal, columns);
        let settings = Selector::settings(
            "fixture/model",
            Effort::Default,
            80,
            &Routing::Auto,
            Default::default(),
            Path::new("C:/fixture/config.json"),
        );
        paint(columns, settings_state(columns, settings), "MCP");
        let mut guided = crate::tui::mcp::PRESETS
            .iter()
            .map(|(name, _, fact)| ((*name).into(), (*fact).into()))
            .collect::<Vec<_>>();
        guided.push(("Server URL".into(), "Any HTTP MCP server".into()));
        let add = Selector::mcp(Panel::Add, "Add connection", Some("MCP"), "", guided);
        paint(columns, settings_state(columns, add), "Vercel");
        let mut empty = Selector::mcp(Panel::List, "MCP", None, "all conversations", vec![]);
        empty.message = "No configured connections · open Settings › MCP".into();
        paint(
            columns,
            settings_state(columns, empty),
            "No configured connections",
        );
        let configured = Selector::mcp(
            Panel::List,
            "MCP",
            Some("Settings"),
            "all conversations",
            vec![
                ("GitHub".into(), "20 tools".into()),
                ("GitHub club".into(), "suspended".into()),
                ("Supabase".into(), "sign in again".into()),
                ("Add connection".into(), String::new()),
            ],
        );
        paint(columns, settings_state(columns, configured), "GitHub club");
        let tools = Selector::mcp(
            Panel::Tools,
            "GitHub",
            Some("MCP"),
            "tools",
            vec![
                ("get_me".into(), "Details of the signed-in user".into()),
                ("search_repositories".into(), "Search repositories".into()),
            ],
        );
        paint(
            columns,
            settings_state(columns, tools),
            "search_repositories",
        );
        let detail = State {
            width: columns,
            height: 30,
            information: Some(Information::new(
                "search_repositories".into(),
                vec![
                    (
                        "Description".into(),
                        "Search repositories by name, topic or owner".into(),
                    ),
                    ("Inputs".into(), r#"{"query":{"type":"string"}}"#.into()),
                ],
            )),
            ..State::default()
        };
        paint(columns, detail, "Inputs");
        for unknown in [false, true] {
            let mut state = State {
                width: columns,
                height: 30,
                ..State::default()
            };
            state.message(Kind::User, "Track this issue on GitHub.");
            state.event(Event::ToolStarted {
                id: "remote-issue".into(),
                name: "mcp".into(),
                arguments: Value::object([
                    ("connection_name", Value::string("GitHub")),
                    ("action", Value::string("call")),
                    ("tool", Value::string("create_issue")),
                    (
                        "arguments",
                        Value::object([("title", Value::string("Fix sleep"))]),
                    ),
                ]),
            });
            let result = if unknown {
                Value::object([
                    ("outcome", Value::string("unknown")),
                    ("error", Value::string("Disconnected after dispatch")),
                ])
            } else {
                Value::object([(
                    "result",
                    Value::object([(
                        "content",
                        Value::Array(vec![Value::object([
                            ("type", Value::string("text")),
                            ("text", Value::string("Created #7")),
                        ])]),
                    )]),
                )])
            };
            state.event(Event::ToolFinished {
                id: "remote-issue".into(),
                name: "mcp".into(),
                summary: crate::mcp::summary(&result),
                result,
            });
            paint(
                columns,
                state,
                if unknown {
                    "Outcome unknown"
                } else {
                    "Created #7"
                },
            );
        }
        println!("MCP_NATIVE_{columns}_VERIFIED");
        io::stdout().flush().unwrap();
    }
    drop(terminal);
}

fn wait_for_width(terminal: &Terminal, expected: usize) {
    if terminal.size.0 == expected {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(super::Input::Size(size)) =
            terminal.input.recv_timeout(Duration::from_millis(200))
            && size.width == expected
        {
            return;
        }
    }
    panic!("ConPTY did not resize to {expected} columns");
}

fn settings_state(columns: usize, selector: Selector) -> State {
    State {
        width: columns,
        height: 30,
        selector: Some(selector),
        ..State::default()
    }
}

fn paint(columns: usize, state: State, expected: &str) {
    let frame = view::frame(&state, "fixture/model", "project");
    assert!(
        frame
            .live
            .iter()
            .any(|line| line.plain().contains(expected)),
        "native frame lacked {expected}"
    );
    write!(
        io::stdout(),
        "{}",
        Renderer::new().paint(&frame, columns, state.height)
    )
    .unwrap();
    io::stdout().flush().unwrap();
    assert!(probe().contains(expected), "ConPTY lacked {expected}");
}
