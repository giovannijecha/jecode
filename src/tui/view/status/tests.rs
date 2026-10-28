use super::*;
use crate::tui::{
    activity::Activity,
    theme::{LIT, S_BASE, S2, SELECTED_SURFACE},
};

fn shown(state: &State, columns: usize) -> Line {
    row(state, columns, true)
}

#[test]
fn idle_is_an_empty_row() {
    assert_eq!(shown(&State::default(), 80).plain(), "");
}

#[test]
fn a_result_is_a_pill_with_a_good_check_and_dim_facts() {
    let mut state = State::default();
    state.notify(Feedback::result(
        Kind::Notice,
        "Copied · 1.2 KiB · response",
    ));
    let line = shown(&state, 80);
    assert_eq!(line.plain(), " ✓  Copied   1.2 KiB · response");
    assert_eq!(style_of(&line, "✓"), Some(GOOD.bold()));
    assert_eq!(style_of(&line, "Copied"), Some(STRONG));
    assert_eq!(style_of(&line, "1.2 KiB"), Some(DIM));
    let pill: Vec<_> = line
        .spans
        .iter()
        .take_while(|span| span.style != DIM)
        .collect();
    assert!(
        pill.iter()
            .all(|span| span.background == Some(PILL_SURFACE) || span.text == "  ")
    );
}

#[test]
fn a_change_reads_old_to_new_with_the_new_value_strong() {
    let mut state = State::default();
    state.notify(
        Feedback::result(
            Kind::Notice,
            "Auto-compact: 80% → 55% · this and new conversations",
        )
        .change(),
    );
    let line = shown(&state, 80);
    assert_eq!(
        line.plain(),
        " ✓  Auto-compact   80% → 55% · this and new conversations"
    );
    assert_eq!(style_of(&line, "80% →"), Some(DIM));
    assert_eq!(style_of(&line, "55%"), Some(STRONG));
    // Without the change flag an arrow is ordinary text.
    state.notify(Feedback::result(
        Kind::Notice,
        "Previous session: Move A → B",
    ));
    let line = shown(&state, 80);
    assert_eq!(line.plain(), " ✓  Previous session   Move A → B");
    assert_eq!(style_of(&line, "Move A → B"), Some(DIM));
}

#[test]
fn warnings_and_errors_keep_their_keycap_two_cells_from_the_edge() {
    let mut state = State::default();
    state.notify(
        Feedback::result(Kind::Warning, "No model · choose one to start")
            .action("/model", "choose"),
    );
    let line = shown(&state, 60);
    assert!(
        line.plain()
            .starts_with(" ▲  No model   choose one to start")
    );
    assert!(
        line.plain().ends_with(" /model  choose"),
        "{}",
        line.plain()
    );
    assert_eq!(text::cells(&line.plain()), 58);
    assert_eq!(style_of(&line, "▲"), Some(WARNING.bold()));
    let key = line
        .spans
        .iter()
        .find(|span| span.text == " /model ")
        .unwrap();
    assert_eq!(
        (key.style, key.background),
        (STRONG, Some(SELECTED_SURFACE))
    );
    // Narrow rows drop the facts first, then the keycap.
    assert_eq!(
        shown(&state, 34).plain(),
        " ▲  No model      /model  choose"
    );
    assert_eq!(shown(&state, 20).plain(), " ▲  No model ");
    state.notify(Feedback::result(
        Kind::Error,
        "Autosave failed: permission denied",
    ));
    let line = shown(&state, 80);
    assert_eq!(line.plain(), " ✕  Autosave failed   permission denied");
    assert_eq!(style_of(&line, "✕"), Some(BAD.bold()));
}

#[test]
fn a_lone_long_fact_shortens_instead_of_dropping() {
    let mut state = State::default();
    state.notify(Feedback::result(
        Kind::Error,
        "Copy failed: the clipboard service refused the request",
    ));
    let line = shown(&state, 40);
    assert_eq!(text::cells(&line.plain()), 38);
    assert!(line.plain().starts_with(" ✕  Copy failed   the clipboard"));
    assert!(line.plain().ends_with('…'));
}

#[test]
fn a_long_label_yields_room_so_the_cause_stays_readable() {
    let mut state = State::default();
    state.notify(Feedback::result(
        Kind::Error,
        "Could not save C:\\Users\\someone\\.jecode\\config.json: Configuration is read-only",
    ));
    let line = shown(&state, 80);
    assert!(text::cells(&line.plain()) <= 78);
    assert!(line.plain().starts_with(" ✕  Could not save C:"));
    assert!(line.plain().ends_with("   Configuration is read-only"));
    // A short label keeps its words; the fact shortens instead.
    state.notify(Feedback::result(
        Kind::Error,
        "OpenRouter HTTP 401: Rejected the key. Check your key with jecode setup.",
    ));
    let rejected = shown(&state, 60).plain();
    assert!(rejected.starts_with(" ✕  OpenRouter HTTP 401   Rejected"));
    assert!(rejected.ends_with('…'));
    // Neither side shrinks below half the row, so the label names the action.
    state.notify(Feedback::result(
        Kind::Error,
        "Could not save C:\\Users\\someone\\.jecode\\config.json: Configuration is read-only",
    ));
    let narrow = shown(&state, 30).plain();
    assert!(narrow.starts_with(" ✕  Could not sa"));
    assert!(text::cells(&narrow) <= 28);
    // A cut too small to let the fact fit keeps the label whole.
    state.notify(Feedback::result(
        Kind::Error,
        "Default provider unavailable · OpenAI has no active ChatGPT account",
    ));
    let close = shown(&state, 57).plain();
    assert!(close.starts_with(" ✕  Default provider unavailable   OpenAI"));
    assert!(close.ends_with('…') && text::cells(&close) <= 55);
}

#[test]
fn turn_endings_stay_until_the_next_send_and_notices_cover_them() {
    let mut state = State::default();
    state.notify(Feedback::ending(Ending::Done, "1m 04s · 4 tools · ↓ 3.2k"));
    let line = shown(&state, 80);
    assert_eq!(line.plain(), " >_ Done   1m 04s · 4 tools · ↓ 3.2k");
    assert_eq!(style_of(&line, ">_"), Some(GOOD.bold()));
    assert!(
        !state.expire_feedback(std::time::Instant::now() + std::time::Duration::from_secs(600))
    );
    state.feedback_input(true);
    assert!(state.ending.is_some());
    // A result covers the ending while it lasts.
    state.notify(Feedback::result(Kind::Notice, "Copied"));
    assert_eq!(shown(&state, 80).plain(), " ✓  Copied ");
    state.notice = None;
    assert!(shown(&state, 80).plain().contains("Done"));
    state.sent();
    assert!(state.ending.is_none());
    assert_eq!(shown(&state, 80).plain(), "");

    state.notify(Feedback::ending(
        Ending::Stopped,
        "6.0s · your next message continues from here",
    ));
    let line = shown(&state, 80);
    assert_eq!(
        line.plain(),
        " ■  Stopped   6.0s · your next message continues from here"
    );
    assert_eq!(style_of(&line, "■"), Some(WARNING));
    state.notify(
        Feedback::ending(Ending::Failed, "2.1s · prompt restored")
            .action("/settings", "update key"),
    );
    let line = shown(&state, 80);
    assert!(
        line.plain()
            .starts_with(" ✕  Failed   2.1s · prompt restored")
    );
    assert!(line.plain().ends_with(" /settings  update key"));
}

#[test]
fn errors_wait_for_the_next_send_and_warnings_for_the_next_key() {
    let mut state = State::default();
    state.notify(Feedback::result(Kind::Error, "Autosave failed"));
    state.feedback_input(false);
    assert!(state.notice.is_some());
    state.sent();
    assert!(state.notice.is_none());
    state.notify(Feedback::result(Kind::Warning, "No model"));
    state.feedback_input(false);
    assert!(state.notice.is_none());
}

#[test]
fn the_working_label_shimmers_and_the_caret_blinks_with_the_shared_frame() {
    let mut state = State {
        activity: Some(Activity::new()),
        ..State::default()
    };
    state.status = "Thinking…".into();
    motion::set_frame(4);
    let line = shown(&state, 80);
    // Frame 4 puts the band's centre on the label's first cell and turns the caret off.
    assert!(
        line.plain().starts_with(" >  Thinking "),
        "{}",
        line.plain()
    );
    assert_eq!(style_of(&line, "T"), Some(S2));
    assert_eq!(style_of(&line, "king"), Some(S_BASE));
    motion::set_frame(0);
    let line = shown(&state, 80);
    assert!(line.plain().starts_with(" >_ Thinking "));
    assert_eq!(style_of(&line, "_"), Some(LIT.bold()));
}

#[test]
fn progress_outside_a_turn_uses_the_caret_and_its_subject() {
    let mut state = State::default();
    state.notify(Feedback::progress(
        Kind::Notice,
        "Compacting conversation · 42 messages",
    ));
    assert_eq!(
        shown(&state, 80).plain(),
        " >_ Compacting conversation   42 messages"
    );
}

#[test]
fn tool_labels_prefer_the_description_then_a_verb_and_a_short_target() {
    let arguments = |pairs: &[(&str, &str)]| {
        Value::Object(
            pairs
                .iter()
                .map(|(key, value)| ((*key).into(), Value::string(*value)))
                .collect(),
        )
    };
    for (name, pairs, label) in [
        (
            "bash",
            vec![
                ("command", "cargo test auth"),
                ("description", "Running auth tests"),
            ],
            "Running auth tests",
        ),
        (
            "bash",
            vec![("command", "cargo test auth\necho done")],
            "Running cargo test auth",
        ),
        (
            "read",
            vec![("path", "src/tui/view/status.rs")],
            "Reading status.rs",
        ),
        (
            "read",
            vec![("path", "skill:review/SKILL.md")],
            "Loading skill review",
        ),
        (
            "edit",
            vec![("path", r"C:\repo\main.rs")],
            "Editing main.rs",
        ),
        ("write", vec![("path", "NOTES.md")], "Writing NOTES.md"),
        ("list", vec![], "Listing ."),
        ("list", vec![("path", "src/tools")], "Listing tools"),
        (
            "search",
            vec![("query", "Cancellation")],
            "Searching Cancellation",
        ),
        (
            "search",
            vec![
                ("query", "needle"),
                ("description", "Find cancellation handling"),
            ],
            "Find cancellation handling",
        ),
        (
            "web",
            vec![("action", "fetch"), ("url", "https://docs.rs/serde?x=1")],
            "Fetching docs.rs",
        ),
        (
            "browser",
            vec![("action", "open"), ("url", "http://localhost:3000/")],
            "Browsing localhost:3000",
        ),
        (
            "process",
            vec![("action", "poll"), ("process_id", "p3")],
            "Polling p3",
        ),
        ("process", vec![("action", "list")], "Listing processes"),
        (
            "checkpoint",
            vec![("action", "save")],
            "Saving a checkpoint",
        ),
        ("custom", vec![], "Running custom"),
    ] {
        assert_eq!(tool_label(name, &arguments(&pairs)), label);
    }
    let long = "x".repeat(60);
    assert_eq!(
        text::cells(&tool_label("bash", &arguments(&[("command", &long)]))),
        8 + TARGET
    );
}

#[test]
fn multiple_live_tools_show_a_count_until_only_one_remains() {
    use crate::events::Event;
    let mut state = State {
        activity: Some(Activity::new()),
        ..State::default()
    };
    for (id, path) in [("a", "src/first.rs"), ("b", "src/second.rs")] {
        state.event(Event::ToolStarted {
            id: id.into(),
            name: "read".into(),
            arguments: Value::object([("path", Value::string(path))]),
        });
    }
    assert_eq!(verb(&state), "2 tools running");
    for columns in [40, 80] {
        let line = shown(&state, columns);
        assert!(line.plain().contains("2 tools running"), "{}", line.plain());
        assert!(text::cells(&line.plain()) <= columns);
    }
    state.event(Event::ToolFinished {
        id: "b".into(),
        name: "read".into(),
        summary: "done".into(),
        result: Value::object([]),
    });
    assert_eq!(verb(&state), "Reading first.rs");
}
