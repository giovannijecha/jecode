//! The `$` skill and `@` file sheets and reference chips, driven by the same input as
//! the terminal and painted through the actual Windows ConPTY renderer, narrow and
//! wide, in both palettes. One renderer paints every frame, across palettes and the
//! console's resizes, and the console's own cells and cursor are read back after
//! every step.
use super::{Terminal, native_mcp::wait_for_width, probe};
use crate::{
    agent::Agent,
    attachments::Attachment,
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
    session::SessionConfig,
    test_support::Directory,
    tools::Tools,
    tui::{App, Decoded, render::Renderer, terminal::Key, test_screen::Screen, theme, view},
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
        Duration::from_secs(240),
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
    let mut native = Native::default();
    let mut sent = None;
    for columns in [44, 80, 100] {
        wait_for_width(&terminal, columns);
        for light in [false, true] {
            if theme::set_light(light) {
                native.renderer.repaint();
            }
            flow(&mut native, &home, &project, columns);
            super::native_reference_edits::edits(&mut native, &home, &project, columns);
            super::native_reference_edits::edge_cases(&mut native, &home, &project, columns);
            let sent = sent.get_or_insert_with(|| {
                super::native_reference_edits::send(&mut native, &home, &project, columns)
            });
            super::native_reference_edits::sent(&mut native, sent, columns);
        }
        if theme::set_light(false) {
            native.renderer.repaint();
        }
        println!("REFERENCES_NATIVE_{columns}_VERIFIED");
        io::stdout().flush().unwrap();
    }
    drop(sent);
    drop(terminal);
}

/// A draft with an attachment gains a skill and a nested file, then a cancelled `$`.
fn flow(native: &mut Native, home: &Directory, project: &Directory, columns: usize) {
    let mut app = app(home, project, columns);
    app.state.editor.insert("Review ");
    app.state.editor.attach(image());
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
        native.paint(&app, expected);
    }
    text(&mut app, "rev");
    native.paint(&app, "1 of 2 skills");
    press(&mut app, 13);
    assert!(app.state.selector.is_none());
    native.paint(&app, "$review");
    native.chip(&app, " $review ");

    text(&mut app, "@");
    settle(&mut app);
    one_caret(&app);
    for expected in ["─ Files", "docs/", "README.md", "Enter open"] {
        native.paint(&app, expected);
    }
    press(&mut app, 40);
    press(&mut app, 13);
    settle(&mut app);
    native.paint(&app, "tui/");
    native.paint(&app, "main.rs");
    text(&mut app, "input");
    native.paint(&app, "tui/input.rs");
    press(&mut app, 9);
    assert!(app.state.selector.is_none());
    assert!(
        app.state
            .editor
            .text
            .ends_with("$review @src/tui/input.rs ")
    );
    // The composer wraps the path at 44 columns; its text is checked above.
    native.paint(&app, "with  $review");
    native.chip(&app, " @src/tui/input.rs ");

    text(&mut app, "@");
    settle(&mut app);
    text(&mut app, "my file");
    native.paint(&app, "my file.md");
    press(&mut app, 13);
    assert!(app.state.selector.is_none());
    native.chip(&app, " @\"docs/my file.md\" ");

    text(&mut app, "$");
    text(&mut app, "zzz");
    native.paint(&app, "No matching skill");
    let before = app.state.editor.text.clone();
    press(&mut app, 13);
    assert!(
        app.state.selector.is_some(),
        "an empty result selects nothing"
    );
    press(&mut app, 27);
    assert_eq!(app.state.editor.text, before);
    assert_eq!(app.state.editor.attachments.len(), 1);
    native.paint(&app, "1 Image");
}

pub(super) fn image() -> Attachment {
    Attachment {
        id: "att-native".into(),
        name: "shot.png".into(),
        media: "image/png".into(),
        size: 10,
        width: Some(1),
        height: Some(1),
        pages: None,
    }
}

pub(super) fn app(home: &Directory, project: &Directory, columns: usize) -> App {
    app_at(
        home,
        project,
        columns,
        "http://127.0.0.1:1/chat/completions",
    )
}

pub(super) fn app_at(home: &Directory, project: &Directory, columns: usize, endpoint: &str) -> App {
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let store = Store::new(home.path().join("config"));
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, SessionConfig { store, settings }, None);
    app.state.width = columns;
    app.state.height = 30;
    app
}

pub(super) fn text(app: &mut App, value: &str) {
    for character in value.chars() {
        app.input(Decoded::Text(character.into())).unwrap();
    }
}

pub(super) fn press(app: &mut App, code: u16) {
    app.input(Decoded::Key(Key {
        code,
        modifiers: 0,
        character: code,
    }))
    .unwrap();
}

pub(super) fn settle(app: &mut App) {
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

/// Paints every frame with one renderer, and keeps a model of the terminal fed the
/// same bytes for the surfaces the console probe cannot read.
#[derive(Default)]
pub(super) struct Native {
    pub renderer: Renderer,
    screen: Option<Screen>,
}

impl Native {
    /// Paints the app's frame and returns the console's visible cells. The console must
    /// show exactly the cells the frame produces, its cursor where the frame's caret
    /// is, and the same renderer's screen must equal a fresh paint of that frame.
    pub fn paint(&mut self, app: &App, expected: &str) -> String {
        let frame = self.render(app, expected);
        let (width, height) = (app.state.width, app.state.height);
        let screen = self.screen.as_ref().unwrap();
        let snapshot = probe();
        let mut lines = snapshot.lines().skip(1);
        let cells: Vec<char> = lines.next().unwrap().chars().collect();
        assert_eq!(cells.len(), width * height, "console size");
        for (row, model) in screen.rows.iter().enumerate() {
            let native: String = cells[row * width..(row + 1) * width].iter().collect();
            let model: String = model.iter().collect();
            assert_eq!(
                native.replace('\u{a0}', " "),
                model.replace('\u{a0}', " "),
                "ConPTY row {row} after {expected}"
            );
        }
        if let Some((row, column)) = frame.cursor {
            let expected_cursor = format!(
                "{},{}",
                column.min(width - 1),
                row.min(height.saturating_sub(1))
            );
            assert_eq!(
                lines.next(),
                Some(expected_cursor.as_str()),
                "ConPTY cursor"
            );
        }
        let visible: String = cells.into_iter().collect();
        assert!(
            visible.replace('\u{a0}', " ").contains(expected),
            "ConPTY lacked {expected}"
        );
        visible
    }

    /// Paints the app's frame with the one renderer, and checks the bytes left no
    /// stale cell or surface against a fresh paint, without reading the console.
    pub fn render(&mut self, app: &App, expected: &str) -> view::Frame {
        let state = &app.state;
        let (width, height) = (state.width, state.height);
        let frame = view::frame(state, "fixture/model", "project");
        assert!(
            frame
                .live
                .iter()
                .any(|line| line.plain().replace('\u{a0}', " ").contains(expected)),
            "Rust frame lacked {expected}"
        );
        let output = self.renderer.paint(&frame, width, height);
        write!(io::stdout(), "{output}").unwrap();
        io::stdout().flush().unwrap();
        let screen = self
            .screen
            .get_or_insert_with(|| Screen::new(width, height));
        if (screen.rows[0].len(), screen.rows.len()) != (width, height) {
            screen.resize(width, height);
        }
        screen.feed(&output);
        let mut fresh = Screen::new(width, height);
        fresh.feed(&Renderer::new().paint(&frame, width, height));
        assert_eq!(screen.rows, fresh.rows, "stale cells after {expected}");
        assert_eq!(screen.backs, fresh.backs, "stale surfaces after {expected}");
        frame
    }

    /// Check the same filled surface as attachments before reading the native cells.
    pub fn chip(&mut self, app: &App, expected: &str) {
        let frame = view::frame(&app.state, "fixture/model", "project");
        assert!(
            frame.live.iter().flat_map(|line| &line.spans).any(|span| {
                span.text.replace('\u{a0}', " ") == expected
                    && span.style == theme::LIT
                    && span.background == Some(theme::BADGE_IDLE)
            }),
            "reference lacked its attachment surface: {expected}"
        );
        let shown = self.paint(app, expected.trim());
        assert!(
            shown.replace('\u{a0}', " ").contains(expected),
            "ConPTY lacked the chip {expected}"
        );
    }
}
