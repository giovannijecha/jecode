//! Reference chips edited and sent in the native console: movement, Backspace, Delete,
//! typing beside a chip, Tab and Esc in the composer, then the sent request in its
//! user box at every width and palette. Each step goes through the same renderer.

use super::native_references::{Native, app, app_at, image, press, settle, text};
use crate::{
    json::Value,
    test_support::{Directory, HttpFixture, completion},
    tui::{App, Decoded, references::syntax, theme, view},
};
use std::time::{Duration, Instant};

const LONG: &str = "@\"docs/a very long folder name/with a long file name for the transcript.md\"";

fn after_caret(app: &App) -> &str {
    &app.state.editor.text[app.state.editor.cursor..]
}

/// The composer's rows among the console cells: from its `› ` row to the rule below.
/// Elsewhere the empty screen may list earlier conversations that name references.
fn composer(shown: &str, width: usize) -> String {
    let cells: Vec<char> = shown.chars().collect();
    let rows: Vec<String> = cells
        .chunks(width)
        .map(|row| row.iter().collect())
        .collect();
    let start = rows.iter().rposition(|row| row.starts_with("› ")).unwrap();
    rows[start..]
        .iter()
        .take_while(|row| !row.starts_with('─'))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Paints the step through the one renderer, and reads the console back whenever the
/// caret stands at a chip's start or end.
fn step(native: &mut Native, app: &App) {
    let editor = &app.state.editor;
    let mut boundary = false;
    for chip in syntax::chips(&editor.text) {
        assert!(!(chip.start < editor.cursor && editor.cursor < chip.end));
        boundary |= editor.cursor == chip.start || editor.cursor == chip.end;
    }
    if boundary {
        native.paint(app, "Check");
    } else {
        native.render(app, "Check");
    }
}

/// Moves with `code` until the text after the caret starts with `prefix`, painting
/// every step.
fn move_to(native: &mut Native, app: &mut App, code: u16, prefix: &str) {
    for _ in 0..200 {
        if after_caret(app).starts_with(prefix) {
            return;
        }
        press(app, code);
        step(native, app);
    }
    panic!("the caret never reached {prefix}");
}

pub(super) fn edits(native: &mut Native, home: &Directory, project: &Directory, columns: usize) {
    let mut app = app(home, project, columns);
    app.state.editor.insert("Check ");
    app.state.editor.attach(image());
    let words = " with @AGENTS.md and $accessibility, then @\"docs/my file.md\" ok";
    app.input(Decoded::Paste(words.into())).unwrap();
    for chip in [
        " 1 Image ",
        " @AGENTS.md ",
        " $accessibility ",
        " @\"docs/my file.md\" ",
    ] {
        native.chip(&app, chip);
    }
    // Every Left and Right crosses a chip whole and repaints only what changed.
    while app.state.editor.cursor > 0 {
        press(&mut app, 37);
        step(native, &app);
    }
    while app.state.editor.cursor < app.state.editor.text.len() {
        press(&mut app, 39);
        step(native, &app);
    }
    move_to(native, &mut app, 37, ", then");
    press(&mut app, 8);
    let shown = composer(&native.paint(&app, "Check"), columns);
    assert!(
        !shown.contains("$accessibility"),
        "Backspace left part of a chip"
    );
    native.chip(&app, " @AGENTS.md ");

    press(&mut app, 36);
    move_to(native, &mut app, 39, "@AGENTS");
    press(&mut app, 46);
    let shown = composer(&native.paint(&app, "Check"), columns);
    assert!(!shown.contains("@AGENTS"), "Delete left part of a chip");
    native.chip(&app, " 1 Image ");

    // Text typed beside the quoted chip keeps it whole, and Tab on its end reopens it.
    press(&mut app, 35);
    move_to(native, &mut app, 37, " ok");
    text(&mut app, "x");
    native.chip(&app, " @\"docs/my file.md\" ");
    press(&mut app, 37);
    let before = app.state.editor.clone();
    press(&mut app, 9);
    settle(&mut app);
    native.paint(&app, "─ Files");
    press(&mut app, 27);
    assert_eq!(app.state.editor, before);
    native.chip(&app, " @\"docs/my file.md\" ");
}

/// Incomplete quoted paths cannot capture attachments, and Unicode spaces never
/// split sent reference cards while the same renderer changes its screen.
pub(super) fn edge_cases(
    native: &mut Native,
    home: &Directory,
    project: &Directory,
    columns: usize,
) {
    let mut app = app(home, project, columns);
    text(&mut app, "Check @");
    press(&mut app, 27);
    text(&mut app, "\"docs/");
    app.state.editor.attach(image());
    app.state.editor.insert(" ");
    text(&mut app, "file.md\" ");
    app.state.editor.attach(image());
    native.chip(&app, " 1 Image ");
    native.chip(&app, " 2 Image ");
    app.state.editor.take();
    app.state.message(
        crate::tui::state::Kind::User,
        "Check something @\"docs/my\u{2003}long file name.md\" now",
    );
    native.chip(&app, " @\"docs/my\u{2003}long file name.md\" ");
}

/// Sends a request with an image and references to an isolated loopback endpoint,
/// which answers once. The model receives the original text and only the image.
pub(super) fn send(
    native: &mut Native,
    home: &Directory,
    project: &Directory,
    columns: usize,
) -> App {
    let fixture = HttpFixture::new(vec![(200, completion("Done.", vec![]))]);
    let mut app = app_at(home, project, columns, &fixture.endpoint);
    let pool = app.persistence.as_ref().unwrap().store().attachments();
    let shot = pool
        .import_bytes("shot.png", &crate::attachments::tests::png(2, 2))
        .unwrap();
    app.state.editor.insert("Check ");
    app.state.editor.attach(shot);
    let words = format!(" with @AGENTS.md, $review and {LONG} please");
    app.input(Decoded::Paste(words.clone())).unwrap();
    native.chip(&app, " @AGENTS.md ");
    press(&mut app, 13);
    assert!(app.state.editor.text.is_empty());
    native.paint(&app, "Check");
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.worker.is_some() {
        assert!(Instant::now() < deadline, "the turn did not end");
        app.poll().unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    let requests = fixture.finish();
    let user = requests[0]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|message| message.encode().contains("image_url"))
        .unwrap()
        .encode();
    assert!(user.contains(&words.replace('"', "\\\"")), "{user}");
    assert!(!user.contains('…') && !user.contains('\u{a0}'));
    app
}

/// The sent request keeps its chips in the user box, the long reference shortened only
/// where the row is too narrow.
pub(super) fn sent(native: &mut Native, app: &mut App, columns: usize) {
    app.state.width = columns;
    for chip in [" 1 Image ", " @AGENTS.md ", " $review "] {
        native.chip(app, chip);
    }
    let frame = view::frame(&app.state, "fixture/model", "project");
    let long = frame
        .live
        .iter()
        .flat_map(|line| &line.spans)
        .find(|span| {
            span.text.starts_with("\u{a0}@\"docs/")
                && span.style == theme::LIT
                && span.background == Some(theme::BADGE_IDLE)
        })
        .expect("the long reference chip")
        .text
        .replace('\u{a0}', " ");
    // The request wraps at four cells less than the console.
    if LONG.len() + 2 > columns - 4 {
        assert!(
            long.ends_with("… ") && long.chars().count() <= columns - 4,
            "{long}"
        );
    } else {
        assert_eq!(long, format!(" {LONG} "));
    }
    let shown = native.paint(app, "Check");
    assert!(shown.replace('\u{a0}', " ").contains(&long));
    let user = frame
        .live
        .iter()
        .find(|line| line.plain().starts_with("› Check"))
        .expect("the request row");
    assert_eq!(user.background, Some(theme::USER_SURFACE));
}
