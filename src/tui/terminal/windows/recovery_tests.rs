use super::super::super::{App, terminal::Terminal};
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
    json::Value,
    session::SessionConfig,
    test_support::HttpFixture,
    tools::Tools,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub(super) fn fixture(path: &Path) {
    let fixture = HttpFixture::new(vec![(
        404,
        Value::object([(
            "error",
            Value::object([("message", Value::string("No image endpoint"))]),
        )]),
    )]);
    let config = SessionConfig {
        store: Store::new(path.join("home")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
    };
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(path).unwrap(),
    );
    agent.enable_sessions(&path.join("home")).unwrap();
    let terminal = Terminal::open().unwrap();
    let mut app = App::new(agent, config, Some(terminal));
    app.state.editor.insert("NATIVE_RECOVERED_REQUEST");
    app.submit().unwrap();
    app.renderer
        .draw(&app.state, "fixture/model", "fixture directory")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    while app.worker.is_some() {
        app.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    app.renderer
        .draw(&app.state, "fixture/model", "fixture directory")
        .unwrap();
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n'@\n[JecodeConsoleProbe]::Snapshot()",
        include_str!("native_probe.cs")
    );
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let screen = String::from_utf8(output.stdout).unwrap();
    let cells = screen.lines().nth(1).unwrap();
    assert_eq!(
        cells.matches("NATIVE_RECOVERED_REQUEST").count(),
        1,
        "{screen}"
    );
    let index = cells.find("NATIVE_RECOVERED_REQUEST").unwrap();
    assert!(
        index >= app.state.width * (app.state.height - 6),
        "prompt stayed in transcript: {screen}"
    );
    assert_eq!(app.state.editor.text, "NATIVE_RECOVERED_REQUEST");
    drop(app);
    assert_eq!(fixture.finish().len(), 1);
}

/// The adapter dies while the TUI owns the console, then a later session fails
/// a handled startup step: each restores modes and the title exactly once.
pub(super) fn guardian_loss(path: &Path) {
    use super::native_tests::{restored, title};
    let mut terminal = Terminal::open().unwrap();
    assert_eq!(title(), "Jecode");
    terminal.close_for_fixture();
    assert!(terminal.check().is_err());
    // The killed adapter could not restore the title; recovery must.
    assert_eq!(title(), "Jecode");
    drop(terminal);
    restored(path);
    let fixture = HttpFixture::new(vec![]);
    let config = SessionConfig {
        store: Store::new(path.join("home")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
    };
    config.store.save(&config.settings).unwrap();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(path).unwrap(),
    );
    let resumed = crate::tui::launch(
        agent,
        config,
        Some(Some("missing-conversation".into())),
        Default::default(),
        None,
        Default::default(),
    );
    assert!(resumed.is_err(), "missing conversation resumed");
    restored(path);
    assert!(fixture.finish().is_empty());
}
