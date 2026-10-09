use super::*;
use crate::{
    agent::Agent,
    cancel::Cancellation,
    config::{Settings, Store},
    openrouter::OpenRouter,
    process,
    session::SessionConfig,
    test_support::{Directory, HttpFixture, completion},
    tools::Tools,
};

#[test]
#[ignore = "requires Windows ConPTY for an isolated native console check"]
fn native_windows_fullscreen_controls_and_terminal_lifetime() {
    for scenario in ["interactive", "owner-death", "guardian-loss"] {
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
            .map(|row| crate::tui::line::Line::new(row, "0"))
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
    full_width_console(&bash);
    restored(&path);
    let response = (0..100)
        .map(|i| format!("Native row {i:03}\n"))
        .collect::<String>();
    let fixture = HttpFixture::new(vec![(200, completion(&response, vec![]))]);
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
    crate::tui::run(agent, config).unwrap();
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
    assert_eq!(saved.input.draft.text, "kept draft β🙂");
    assert_eq!(fixture.finish().len(), 1);
    println!("NATIVE_RESTORED");
}
