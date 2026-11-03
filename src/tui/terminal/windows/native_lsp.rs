//! B15 settings, inspection and cards through the real Windows ConPTY renderer.
use super::{Terminal, probe};
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    config::{Settings, Store as SettingsStore},
    events::Event,
    json::Value,
    lsp::{Store as LspStore, fixture::Fixture},
    session::SessionConfig,
    test_support::{Directory, HttpFixture},
    tools::Tools,
    tui::{
        App, Decoded, Kind, display::Display, render::Renderer, selector::Setting, state::State,
        terminal::Key, transcript::Layout, view,
    },
};
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires Windows ConPTY and PowerShell for native LSP panels"]
fn native_windows_lsp_panels() {
    let directory = Directory::new();
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n'@\n[JecodeTestPty]::Run('{}', '{}', 'lsp-panels')\n",
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
        Duration::from_secs(90),
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
    let fixture = Fixture::new();
    let provider = HttpFixture::new(vec![]);
    let home = fixture.workspace().join("home");
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let store = SettingsStore::new(home.clone());
    store.save(&settings).unwrap();
    let mut app = App::new(
        Agent::new(
            OpenRouter::fixture(provider.endpoint.clone()),
            Tools::new(fixture.workspace()).unwrap(),
        ),
        SessionConfig { store, settings },
        None,
    );
    let lsp_store = LspStore::new(home);

    wait_for_width(&terminal, 44);
    app.state.width = 44;
    app.state.height = 30;
    app.open_settings();
    paint_app(&app, "LSP");
    let settings = app.state.selector.as_mut().unwrap();
    settings.selected = settings.setting_index(Setting::Lsp);
    press(&mut app, 13);
    paint_app(&app, "Add server");
    press(&mut app, 13);
    paint_app(&app, "Rust / rust-analyzer");
    press(&mut app, 13);
    paint_app(&app, "Languages");
    assert!(
        app.state
            .selector
            .as_ref()
            .unwrap()
            .options
            .iter()
            .any(|row| row.fact == "On")
    );
    app.state.selector.as_mut().unwrap().selected = 2;
    press(&mut app, 13);
    let prefilled = app
        .state
        .selector
        .as_ref()
        .unwrap()
        .editor
        .text
        .chars()
        .count();
    for _ in 0..prefilled {
        press(&mut app, 8); // Backspace through the guided languages.
    }
    app.input(Decoded::Text("c: .c .h; cpp: .cc .cpp .hpp".into()))
        .unwrap();
    press(&mut app, 13);
    paint_app(&app, "cpp: .cc");
    press(&mut app, 27); // Unsaved form returns to Add.
    assert!(lsp_store.load().unwrap().is_empty());
    press(&mut app, 13); // Begin guided Rust again.
    app.state.selector.as_mut().unwrap().selected = 6;
    press(&mut app, 13); // Save without running a server.
    assert_eq!(lsp_store.load().unwrap().len(), 1);
    assert!(!fixture.marker().exists());
    assert!(app.facts().lsp_runtime.statuses().is_empty());

    let mut profile = fixture.profile();
    profile.name = "Fixture LSP".into();
    lsp_store.save(profile.clone()).unwrap();
    app.state.editor.replace("/lsp".into());
    app.submit().unwrap();
    assert!(app.state.editor.text.is_empty());
    paint_app(&app, "Fixture LSP");
    assert!(!fixture.marker().exists()); // Read-only inspection launches nothing.
    select(&mut app, "Fixture LSP");
    press(&mut app, 13);
    paint_app(&app, "Not checked");
    press(&mut app, 27); // Back to the read-only /lsp list, not its instance.
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        crate::tui::selector::Purpose::Lsp(crate::tui::lsp::Panel::List)
    ));
    paint_app(&app, "Fixture LSP");

    app.open_lsp(true);
    select(&mut app, "Fixture LSP");
    press(&mut app, 13);
    app.state.selector.as_mut().unwrap().selected = 2;
    press(&mut app, 13); // Explicit Check starts one temporary server.
    paint_app(&app, "Starting");
    let deadline = Instant::now() + Duration::from_secs(20);
    while lsp_store
        .load()
        .unwrap()
        .iter()
        .find(|p| p.id == profile.id)
        .unwrap()
        .last_check
        .is_none()
    {
        assert!(Instant::now() < deadline, "LSP check did not settle");
        app.poll_lsp();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(fixture.marker().exists());
    assert!(app.facts().lsp_runtime.statuses().is_empty());
    paint_app(&app, "Pull advertised");
    app.state.selector.as_mut().unwrap().selected = 2;
    press(&mut app, 13);
    press(&mut app, 27); // Cancels a later Check; saved observation survives.
    assert!(
        lsp_store
            .load()
            .unwrap()
            .iter()
            .find(|p| p.id == profile.id)
            .unwrap()
            .last_check
            .is_some()
    );

    let mut state = mixed_work(44);
    assert_ctrl_o(&state);
    assert_eq!(state.toggle_tools(), Some((true, 3)));
    assert_mixed(&state);
    paint_state(&state, "references");
    println!("LSP_NATIVE_44_VERIFIED");
    io::stdout().flush().unwrap();

    wait_for_width(&terminal, 100);
    app.state.width = 100;
    paint_app(&app, "Definition advertised");
    let mut state = mixed_work(100);
    assert_ctrl_o(&state);
    assert_eq!(state.toggle_tools(), Some((true, 3)));
    assert_mixed(&state);
    paint_state(&state, "references");
    println!("LSP_NATIVE_100_VERIFIED");
    io::stdout().flush().unwrap();
    assert!(provider.finish().is_empty());
    drop(terminal);
}

fn select(app: &mut App, name: &str) {
    let menu = app.state.selector.as_mut().unwrap();
    menu.selected = menu
        .filtered
        .iter()
        .position(|hit| menu.options[hit.index].name == name)
        .unwrap();
}

fn press(app: &mut App, code: u16) {
    app.input(Decoded::Key(Key {
        code,
        modifiers: 0,
        character: code,
    }))
    .unwrap();
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

fn paint_app(app: &App, expected: &str) {
    let state = &app.state;
    let frame = view::frame(state, "fixture/model", "project");
    assert!(
        frame
            .live
            .iter()
            .any(|line| line.plain().contains(expected)),
        "Rust frame lacked {expected}"
    );
    write!(
        io::stdout(),
        "{}",
        Renderer::new().paint(&frame, state.width, state.height)
    )
    .unwrap();
    io::stdout().flush().unwrap();
    assert!(probe().contains(expected), "ConPTY lacked {expected}");
}

fn paint_state(state: &State, expected: &str) {
    assert!(
        layout_rows(state).iter().any(|row| row.contains(expected)),
        "Rust layout lacked {expected}"
    );
    let mut display = Display::new((state.width, state.height));
    for _ in 0..16 {
        write!(
            io::stdout(),
            "{}",
            display.update(state, "fixture/model", "project", Instant::now())
        )
        .unwrap();
        if !display.needs_draw(state, Instant::now()) {
            break;
        }
    }
    io::stdout().flush().unwrap();
    assert!(probe().contains(expected), "ConPTY lacked {expected}");
}

fn assert_mixed(state: &State) {
    let rows = layout_rows(state).join("\n");
    for fact in ["READ", "MCP", "LSP", "references"] {
        assert!(rows.contains(fact), "expanded Work lost {fact}: {rows}");
    }
}

fn assert_ctrl_o(state: &State) {
    let rows = layout_rows(state);
    assert!(
        rows.iter().any(|row| row.contains("Ctrl+O")),
        "collapsed Work lost Ctrl+O: {}",
        rows.join("\n")
    );
}

fn layout_rows(state: &State) -> Vec<String> {
    let mut layout = Layout::default();
    for _ in 0..16 {
        layout.prepare(state, state.width, 64);
        if layout.ready(state) {
            break;
        }
    }
    assert!(layout.ready(state), "Rust layout did not settle");
    for _ in 0..16 {
        layout.prepare_pending(state, 256);
        if layout.pending_ready(state) {
            break;
        }
    }
    assert!(
        layout.pending_ready(state),
        "Rust pending layout did not settle"
    );
    layout
        .blocks
        .iter()
        .chain(layout.pending_blocks().iter())
        .flat_map(|block| block.iter().map(|line| line.plain()))
        .collect()
}

fn mixed_work(columns: usize) -> State {
    let mut state = State {
        width: columns,
        height: 30,
        ..State::default()
    };
    state.message(
        Kind::User,
        "Find every reference and inspect the workspace.",
    );
    state.begin_tool_turn();
    for (id, name, arguments, result) in [
        (
            "read",
            "read",
            Value::object([("path", Value::string("src/main.rs"))]),
            Value::object([("content", Value::string("fn main() {}"))]),
        ),
        (
            "mcp",
            "mcp",
            Value::object([
                ("connection_name", Value::string("Fixture")),
                ("action", Value::string("list")),
            ]),
            Value::object([("result", Value::object([]))]),
        ),
        (
            "lsp",
            "lsp",
            Value::object([
                ("server", Value::string("lsp-fixture")),
                ("server_name", Value::string("Fixture LSP")),
                ("action", Value::string("references")),
                ("path", Value::string("src/main.rs")),
                ("line", Value::number(1)),
                ("column", Value::number(4)),
            ]),
            Value::object([
                ("status", Value::string("ready")),
                ("locations", Value::Array(vec![])),
                ("complete", Value::Bool(true)),
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
            summary: String::new(),
            result,
        });
    }
    state.finish_tool_turn();
    state
}
