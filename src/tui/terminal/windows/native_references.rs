//! The `$` skill and `@` file sheets, driven by the same input as the terminal and
//! painted through the actual Windows ConPTY renderer, narrow and wide, in both
//! palettes. The console's own cells are read back after every step.
use super::{Terminal, native_mcp::wait_for_width, probe};
use crate::{
    agent::Agent,
    attachments::Attachment,
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
    session::SessionConfig,
    test_support::Directory,
    tools::Tools,
    tui::{App, Decoded, render::Renderer, terminal::Key, theme, view},
};
use std::{
    fs,
    io::{self, Write},
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires Windows ConPTY and PowerShell for native reference sheets"]
fn native_windows_reference_sheets() {
    let directory = Directory::new();
    let executable = std::env::current_exe().unwrap();
    let result = super::native_script::pty(
        super::native_pty_source(),
        "Run",
        &[
            &executable.to_string_lossy(),
            &directory.path().to_string_lossy(),
            "reference-sheets",
        ],
        &directory,
        Duration::from_secs(120),
        false,
    );
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
    let (home, project) = (Directory::new(), Directory::new());
    for (name, description) in [
        ("review", "Inspect a change for defects"),
        ("release", "Prepare a release"),
    ] {
        let package = home.path().join("skills").join(name);
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n"),
        )
        .unwrap();
    }
    for path in [
        "src/main.rs",
        "src/tui/input.rs",
        "docs/my file.md",
        "README.md",
    ] {
        let path = project.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "fixture").unwrap();
    }
    for columns in [44, 80, 100] {
        wait_for_width(&terminal, columns);
        for light in [false, true] {
            theme::set_light(light);
            flow(&home, &project, columns);
        }
        theme::set_light(false);
        println!("REFERENCES_NATIVE_{columns}_VERIFIED");
        io::stdout().flush().unwrap();
    }
    drop(terminal);
}

/// A draft with an attachment gains a skill and a nested file, then a cancelled `$`.
fn flow(home: &Directory, project: &Directory, columns: usize) {
    let mut app = app(home, project, columns);
    app.state.editor.insert("Review ");
    app.state.editor.attach(Attachment {
        id: "att-native".into(),
        name: "shot.png".into(),
        media: "image/png".into(),
        size: 10,
        width: Some(1),
        height: Some(1),
        pages: None,
    });
    app.state.editor.insert(" with ");
    app.edited();

    text(&mut app, "$");
    one_caret(&app);
    for expected in [
        "─ Skills",
        "draft kept",
        "Filter",
        "Inspect a change",
        "Esc cancel",
    ] {
        paint(&app, expected);
    }
    text(&mut app, "rev");
    paint(&app, "1 of 2 skills");
    press(&mut app, 13);
    assert!(app.state.selector.is_none());
    paint(&app, "$review");

    text(&mut app, "@");
    settle(&mut app);
    one_caret(&app);
    for expected in ["─ Files", "docs/", "README.md", "Enter open"] {
        paint(&app, expected);
    }
    press(&mut app, 40);
    press(&mut app, 13);
    settle(&mut app);
    paint(&app, "tui/");
    paint(&app, "main.rs");
    text(&mut app, "input");
    paint(&app, "tui/input.rs");
    press(&mut app, 9);
    assert!(app.state.selector.is_none());
    assert!(
        app.state
            .editor
            .text
            .ends_with("$review @src/tui/input.rs ")
    );
    // The composer wraps the path at 44 columns; its text is checked above.
    paint(&app, "with $review");

    text(&mut app, "$");
    text(&mut app, "zzz");
    paint(&app, "No matching skill");
    let before = app.state.editor.text.clone();
    press(&mut app, 13);
    assert!(
        app.state.selector.is_some(),
        "an empty result selects nothing"
    );
    press(&mut app, 27);
    assert_eq!(app.state.editor.text, before);
    assert_eq!(app.state.editor.attachments.len(), 1);
    paint(&app, "1 Image");
}

fn app(home: &Directory, project: &Directory, columns: usize) -> App {
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let store = Store::new(home.path().join("config"));
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, SessionConfig { store, settings }, None);
    app.state.width = columns;
    app.state.height = 30;
    app
}

fn text(app: &mut App, value: &str) {
    for character in value.chars() {
        app.input(Decoded::Text(character.into())).unwrap();
    }
}

fn press(app: &mut App, code: u16) {
    app.input(Decoded::Key(Key {
        code,
        modifiers: 0,
        character: code,
    }))
    .unwrap();
}

fn settle(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.references_pending() {
        assert!(Instant::now() < deadline, "the file walk did not settle");
        app.poll().unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    app.poll().unwrap();
}

/// The composer is gone and the sheet's filter holds the only caret.
fn one_caret(app: &App) {
    let frame = view::frame(&app.state, "fixture/model", "project");
    let (row, _) = frame.cursor.expect("a caret");
    assert!(frame.live[row].plain().starts_with("  Filter"));
    assert!(
        !frame
            .live
            .iter()
            .any(|line| line.plain().contains("Ask anything"))
    );
}

fn paint(app: &App, expected: &str) {
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
