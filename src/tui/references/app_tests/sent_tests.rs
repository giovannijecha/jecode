//! References stay chips after sending: the submitted request, a queued message
//! delivered into the turn, a resized transcript and a resumed conversation. The
//! model receives the original text, and references never become attachments.

use super::atomic_tests::drawn;
use super::*;
use crate::{
    attachments::Prompt,
    json::Value,
    test_support::{HttpFixture, completion},
    tui::Kind,
};

const LONG: &str = "@\"docs/a very long folder name/with a long file name for the transcript.md\"";

fn sending(home: &Directory, project: &Directory, endpoint: &str) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, tests::config(project), None);
    app.state.width = 120;
    app.state.height = 40;
    app
}

fn finish_turn(app: &mut App) {
    let started = Instant::now();
    while app.worker.is_some() || !app.imports.is_empty() {
        app.poll().unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the turn stalled"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn sent_chips(app: &App) -> Vec<String> {
    drawn(app)
}

#[test]
fn sent_requests_keep_reference_chips_through_delivery_resize_and_resume() {
    let (home, project) = (Directory::new(), Directory::new());
    let fixture = HttpFixture::new(vec![(200, completion("Done.", vec![]))]);
    let mut app = sending(&home, &project, &fixture.endpoint);
    let pool = app.persistence.as_ref().unwrap().store().attachments();
    let image = pool
        .import_bytes("shot.png", &crate::attachments::tests::png(2, 2))
        .unwrap();
    app.state.editor.insert("Check ");
    app.state.editor.attach(image);
    let words = format!(" with @AGENTS.md, $review and {LONG} please");
    assert!(app.state.editor.compose(&words, false));
    key(&mut app, ENTER);
    let long = format!(" {LONG} ");
    let expected = [" 1 Image ", " @AGENTS.md ", " $review ", long.as_str()];
    assert_eq!(sent_chips(&app), expected, "as submitted");
    finish_turn(&mut app);
    assert_eq!(sent_chips(&app), expected, "after the answer");

    // Narrow, the long reference shortens on screen; wide again, it is whole.
    app.state.width = 44;
    let narrow = sent_chips(&app);
    assert_eq!(narrow[..3], expected[..3]);
    assert!(narrow[3].starts_with(" @\"docs/") && narrow[3].ends_with("… "));
    assert!(crate::tui::text::cells(&narrow[3]) <= 40);
    app.state.width = 120;
    assert_eq!(sent_chips(&app), expected, "after resizing back");

    // A queued message the agent takes into a turn shows the same way.
    let queued = Prompt::plain("Then $review @\"docs/my file.md\" too");
    app.state.queue.inbox().with(|entries| {
        entries.push(queued.clone()).unwrap();
        entries.record();
    });
    let accepted = app.state.queue.inbox().accept(|_, _| Ok(())).unwrap();
    assert_eq!(accepted, [queued]);
    app.reconcile_accepted();
    let chips = sent_chips(&app);
    assert_eq!(
        chips[expected.len()..],
        [" $review ", " @\"docs/my file.md\" "]
    );

    // The model saw the original text and one image, with no padding or ellipsis.
    let requests = fixture.finish();
    let user = requests[0]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|message| message.encode().contains("image_url"))
        .unwrap()
        .clone();
    let encoded = user.encode();
    assert!(encoded.contains(&words.replace('"', "\\\"")), "{encoded}");
    assert!(!encoded.contains('…') && !encoded.contains('\u{a0}'));
    let parts = user.get("content").and_then(Value::as_array).unwrap();
    let media = parts
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str) != Some("text"))
        .count();
    assert_eq!(media, 1, "references never become attachments");

    // A resumed conversation draws its requests with the same chips.
    let id = app.persistence.as_ref().unwrap().id();
    drop(app);
    let mut resumed = sending(&home, &project, "http://127.0.0.1:1/chat/completions");
    resumed.resume_session(&id);
    let chips = sent_chips(&resumed);
    for chip in [" @AGENTS.md ", " $review ", long.as_str()] {
        assert!(
            chips.iter().any(|drawn| drawn == chip),
            "{chip} in {chips:?}"
        );
    }
}

#[test]
fn literal_sigils_and_other_messages_keep_their_plain_rendering() {
    let chips = |kind: Kind, text: &str| -> (Vec<String>, String) {
        let mut state = crate::tui::state::State::default();
        state.message(kind, text);
        let lines: Vec<_> = state.rows().into_iter().flatten().collect();
        let chips = lines
            .iter()
            .flat_map(|line| &line.spans)
            .filter(|span| span.background == Some(crate::tui::theme::BADGE_IDLE))
            .map(|span| span.text.replace('\u{a0}', " "))
            .collect();
        (chips, lines.iter().map(|line| line.plain()).collect())
    };
    // An attachment label is a boundary, as its marker is in the composer; touching
    // chips keep one plain cell between them.
    let (found, plain) = chips(
        Kind::User,
        r"Mail a@b.com US$5 \$x $ @ for [1# Image]@AGENTS.md",
    );
    assert_eq!(found, [" 1 Image ", " @AGENTS.md "]);
    assert!(plain.contains("Image\u{a0} \u{a0}@AGENTS"), "{plain}");
    for (kind, text) in [
        (Kind::User, "/model @literal $argument"),
        (Kind::Assistant, "Read @AGENTS.md with $review."),
        (Kind::Error, "Missing @AGENTS.md"),
    ] {
        assert!(chips(kind, text).0.is_empty(), "{text}");
    }
}

#[test]
fn punctuation_cannot_split_a_full_width_sent_reference() {
    let mut state = crate::tui::state::State {
        width: 16,
        ..crate::tui::state::State::default()
    };
    state.message(Kind::User, "@AGENTS.md, next");
    let lines: Vec<_> = state.rows().into_iter().flatten().collect();
    let chips: Vec<_> = lines
        .iter()
        .flat_map(|line| &line.spans)
        .filter(|span| span.background == Some(crate::tui::theme::BADGE_IDLE))
        .map(|span| span.text.replace('\u{a0}', " "))
        .collect();
    assert_eq!(chips, [" @AGENTS.md "]);
}

#[test]
fn unicode_spaces_cannot_split_a_sent_reference() {
    for width in 12..=32 {
        let mut state = crate::tui::state::State {
            width,
            ..crate::tui::state::State::default()
        };
        state.message(Kind::User, "See @\"docs/a\u{2003}b.md\" now");
        let lines: Vec<_> = state.rows().into_iter().flatten().collect();
        let chips = lines
            .iter()
            .flat_map(|line| &line.spans)
            .filter(|span| span.background == Some(crate::tui::theme::BADGE_IDLE))
            .count();
        assert_eq!(chips, 1, "{width}: {lines:?}");
    }
}

#[test]
fn prose_after_a_shortened_reference_keeps_its_original_source_position() {
    for prefix in ["", "日本 ", "[1# Image]", "[1# Image] 日本 "] {
        let request = format!("{prefix}{LONG} continuation after the long reference");
        let expected = request[..request.find("continuation").unwrap()]
            .chars()
            .count();
        let mut state = crate::tui::state::State {
            width: 44,
            ..crate::tui::state::State::default()
        };
        state.message(Kind::User, &request);
        let lines: Vec<_> = state.rows().into_iter().flatten().collect();
        let prose = lines
            .iter()
            .find(|line| line.plain().starts_with("  continuation"))
            .unwrap();
        assert_eq!(prose.origin.unwrap().character, expected, "{prefix}");
    }
}

#[test]
fn resizing_while_reading_after_a_reference_keeps_the_same_prose() {
    use crate::tui::viewport::{Scroll, Viewport};

    let mut state = crate::tui::state::State {
        width: 44,
        ..crate::tui::state::State::default()
    };
    state.message(
        Kind::User,
        &format!(
            "{LONG} continuation after the long reference {}",
            "more words ".repeat(30)
        ),
    );
    let narrow = state.rows();
    let mut viewport = Viewport::default();
    viewport.scroll(Scroll::Start);
    viewport.scroll(Scroll::Rows(1));
    let before = viewport.view(&narrow, &[], 1);
    assert!(before.lines[0].plain().starts_with("  continuation"));
    state.width = 80;
    let wide = state.rows();
    let after = viewport.view(&wide, &[], 1);
    assert!(after.lines[0].plain().starts_with("  continuation"));
    assert_eq!(after.lines[0].origin, before.lines[0].origin);
}
