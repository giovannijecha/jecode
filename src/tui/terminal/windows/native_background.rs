//! Native ConPTY coverage for the per-renderer fallback and the Windows
//! main-screen background hint used when OSC 11 is unanswered.

use super::*;
use std::path::Path;

fn run_host(scenario: &str) {
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
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(!result.timed_out);
}

#[test]
#[ignore = "requires Windows ConPTY for native light fallback rendering"]
fn native_windows_light_background() {
    run_host("background-light");
}

#[test]
#[ignore = "requires Windows ConPTY for native console background detection"]
fn native_windows_background_detection() {
    run_host("background-native");
}

pub(super) fn fixture(path: &Path, scenario: &str) {
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
        // SGR 107 makes the main-screen palette entry white. The native
        // adapter reads that entry before the TUI enters its alternate screen.
        write!(io::stdout(), "\x1b[107;30m").unwrap();
        io::stdout().flush().unwrap();
        let settings =
            Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
        let config = SessionConfig {
            store: Store::new(path.join("home")),
            settings,
        };
        config.store.save(&config.settings).unwrap();
        let fixture = HttpFixture::new(vec![]);
        let agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(path).unwrap(),
        );
        crate::tui::launch(agent, config, None, Default::default(), None).unwrap();
        write!(io::stdout(), "\x1b[0m").unwrap();
        io::stdout().flush().unwrap();
        assert!(fixture.finish().is_empty());
        println!("NATIVE_BACKGROUND_DETECTED_RESTORED");
    }
}
