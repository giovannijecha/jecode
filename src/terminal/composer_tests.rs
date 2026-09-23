use super::*;
use crate::session::{self, End, Event, Failure, Metrics};

fn ready() -> (model::Model, session::Session) {
    let mut model = account::model(
        session::Model::Luna,
        Some(std::path::Path::new("fixture/project")),
    );
    account::event(&mut model, Event::Ready);
    (model, session::tests::ready_fixture())
}
fn command(model: &mut model::Model, session: &mut session::Session, name: &str) {
    account::input(model, Key::Text(name.into()), session);
    account::input(model, Key::Enter, session);
}
fn transcript(model: &model::Model) -> Vec<style::Row> {
    view::frame(model, 80, 24)
        .into_iter()
        .take_while(|r| !r.transient)
        .collect()
}
fn assert_inside(rows: &[style::Row], needle: &str) {
    let rules: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.text.starts_with('─'))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(rules.len(), 2);
    let index = rows.iter().position(|r| r.text.contains(needle)).unwrap();
    assert!(
        rules[0] < index && index < rules[1],
        "{needle} escaped the composer: {rows:?}"
    );
}
fn assert_above(rows: &[style::Row], needle: &str) {
    let rule = rows.iter().position(|r| r.text.starts_with('─')).unwrap();
    let index = rows.iter().position(|r| r.text.contains(needle)).unwrap();
    assert!(index < rule, "{needle} entered the composer: {rows:?}");
    assert!(rows[index].transient);
}

#[test]
fn account_wait_think_and_local_feedback_keep_their_own_sides_of_the_rule() {
    let (mut model, mut session) = ready();
    command(&mut model, &mut session, "Inspect the fixture");
    assert!(model.account.as_ref().unwrap().generating());
    for (event, needle) in [
        (Event::RequestStarted, "Waiting for model"),
        (Event::Thinking, "Thinking"),
    ] {
        account::event(&mut model, event);
        for (width, height) in [(80, 24), (25, 9)] {
            let rows = view::chrome(&model, width, height);
            assert_above(&rows, needle);
            assert_inside(&rows, "Ask anything");
            assert!(rows.len() < height);
        }
    }
    account::input(&mut model, Key::Text("/".into()), &mut session);
    for (width, height) in [(80, 24), (25, 9)] {
        let rows = view::chrome(&model, width, height);
        assert_above(&rows, "Thinking");
        assert_inside(&rows, "/new");
    }
    account::input(&mut model, Key::Enter, &mut session);
    for (width, height) in [(80, 24), (25, 9)] {
        let rows = view::chrome(&model, width, height);
        assert_above(&rows, "Thinking");
        assert_inside(&rows, "Wait for");
    }
    account::input(&mut model, Key::Escape, &mut session);
    model.editor.take();
    account::input(&mut model, Key::Text("next guidance".into()), &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    assert_inside(&view::chrome(&model, 80, 24), "1 queued");
    account::event(&mut model, Event::GuidanceReturned("next guidance".into()));
    account::event(
        &mut model,
        Event::Finished(End::Complete, Metrics::default()),
    );
    let rows = view::frame(&model, 80, 24);
    assert!(!rows.iter().any(|r| r.text.contains("Thinking")));
    assert!(!rows.iter().any(|r| r.text.contains("Waiting for model")));
    assert_eq!(
        rows.iter()
            .filter(|r| r.text.contains("Queued message was not sent"))
            .count(),
        1
    );
}

#[test]
fn account_completion_failure_and_cancel_clear_activity_without_duplicate_results() {
    for (end, outcome) in [
        (End::Complete, "Explored workspace"),
        (
            End::Failed(Failure::Worker),
            "Account worker stopped unexpectedly",
        ),
        (
            End::Failed(Failure::Cancelled),
            "Interrupted / partial output retained",
        ),
    ] {
        let (mut model, mut session) = ready();
        command(&mut model, &mut session, "Inspect a file");
        account::event(
            &mut model,
            Event::ToolStarted {
                name: "read_file",
                path: "fixture.rs".into(),
            },
        );
        assert_above(&view::chrome(&model, 80, 24), "Exploring workspace");
        model.account.as_mut().unwrap().local_notice = "Wait for the current operation".into();
        if end == End::Complete {
            account::event(
                &mut model,
                Event::ToolFinished {
                    summary: "12 lines".into(),
                    failed: false,
                    limited: false,
                },
            );
        }
        account::event(&mut model, Event::Finished(end, Metrics::default()));
        let rows = view::frame(&model, 80, 24);
        assert!(
            !rows
                .iter()
                .any(|row| row.text.contains("Wait for the current operation"))
        );
        assert!(
            !rows
                .iter()
                .any(|row| row.text.contains("Exploring workspace"))
        );
        assert_eq!(
            rows.iter().filter(|row| row.text.contains(outcome)).count(),
            1,
            "{rows:?}"
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.text.contains(if end == End::Complete {
                    "Explored workspace"
                } else {
                    "Exploration interrupted"
                }))
                .count(),
            1,
            "{rows:?}"
        );
    }
}

#[test]
fn startup_has_only_the_brand_and_current_metadata_below_the_composer() {
    let (mut model, _) = ready();
    assert!(model.blocks.is_empty());
    let header = transcript(&model);
    let text: Vec<_> = header
        .iter()
        .filter(|r| !r.text.is_empty())
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(text, ["jecode"]);
    for selected in [session::Model::Luna, session::Model::Terra] {
        account::event(&mut model, Event::ModelChanged(selected));
        for (width, height) in [(25, 9), (48, 12), (120, 36)] {
            let rows = view::chrome(&model, width, height);
            let last_rule = rows.iter().rposition(|r| r.text.starts_with('─')).unwrap();
            assert_eq!(rows.len() - last_rule - 1, 1, "one footer row: {rows:?}");
            for needle in ["project", "medium"] {
                assert!(
                    rows[last_rule + 1..]
                        .iter()
                        .any(|r| r.text.contains(needle)),
                    "{needle}: {rows:?}"
                );
            }
            if width >= 48 {
                assert!(rows.last().unwrap().text.contains(selected.id()));
            }
            assert_eq!(transcript(&model), header);
        }
    }
}

#[test]
fn command_menu_contains_only_aligned_commands_and_dismisses_without_transcript_changes() {
    let (mut model, mut session) = ready();
    model.blocks.push(model::Block {
        speaker: "Assistant",
        text: "Saved answer.".into(),
    });
    let before = transcript(&model);
    model.account.as_mut().unwrap().notice = "Complete / 1.0s".into();
    account::input(&mut model, Key::Text("/".into()), &mut session);
    assert!(model.account.as_ref().unwrap().notice.is_empty());
    let menu = menu::rows(&model, 79, 12);
    let labels: Vec<_> = menu.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        labels,
        [
            "› /new",
            "  /resume",
            "  /model",
            "  /settings",
            "  /context",
            "  /compact",
            "  /help"
        ]
    );
    assert_inside(&view::chrome(&model, 80, 24), "/resume");
    for _ in 0..6 {
        account::input(&mut model, Key::Down, &mut session);
    }
    assert!(
        menu::rows(&model, 24, 2)
            .last()
            .unwrap()
            .text
            .contains("› /help")
    );
    account::input(&mut model, Key::Escape, &mut session);
    assert_eq!(model.editor.text, "/");
    assert_eq!(transcript(&model), before);
    assert!(session.ready());
}

#[test]
fn command_results_enter_scrollback_once_and_leave_a_clean_composer() {
    let (mut model, mut session) = ready();
    let mut count = 0;
    for name in ["/context", "/compact"] {
        command(&mut model, &mut session, name);
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(event) = session.poll() {
                let done = if name == "/context" {
                    matches!(event, Event::ContextReport(_))
                } else {
                    matches!(event, Event::Finished(..))
                };
                account::event(&mut model, event);
                if done {
                    break;
                }
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        count += 1;
        assert_eq!(model.blocks.len(), count);
        assert_eq!(model.blocks.last().unwrap().speaker, "Status");
        assert!(!model.blocks.last().unwrap().text.is_empty());
        assert!(model.editor.text.is_empty());
        assert!(model.account.as_ref().unwrap().notice.is_empty());
        assert_eq!(view::chrome(&model, 80, 24).len(), 4);
    }
    command(&mut model, &mut session, "/help");
    count += 1;
    assert_eq!(model.blocks.len(), count);
    assert!(!model.blocks.last().unwrap().text.contains("/quit"));
    assert!(session.ready(), "help must not contact the model");
    let before = transcript(&model);
    model.editor.insert("saved draft 👩‍💻");
    account::input(&mut model, Key::Escape, &mut session);
    assert_eq!(model.editor.text, "saved draft 👩‍💻");
    assert_eq!(transcript(&model), before);
    account::compacting(&mut model);
    account::event(
        &mut model,
        Event::Finished(End::Failed(Failure::Cancelled), Metrics::default()),
    );
    assert_eq!(model.blocks.len(), count + 1);
    assert_eq!(model.blocks.last().unwrap().speaker, "Error");
    assert!(model.account.as_ref().unwrap().notice.is_empty());
    let before = transcript(&model);
    account::event(&mut model, Event::ModelChanged(session::Model::Terra));
    let rows = view::chrome(&model, 80, 24);
    assert!(rows.last().unwrap().text.contains("gpt-5.6-terra"));
    assert_eq!(transcript(&model), before);
    model.editor.insert("/");
    account::input(&mut model, Key::Quit, &mut session);
    assert!(model.quit, "Ctrl+Q exits even with local controls open");
}

#[test]
fn panels_remain_bounded_with_long_paths_drafts_and_untrusted_labels() {
    let (mut model, _) = ready();
    model.account.as_mut().unwrap().workspace =
        Some(format!("{}project-中文", "parent/".repeat(60)));
    model.editor.insert(&"draft 👩‍💻 ".repeat(40));
    let mut panel = menu::models(session::Model::Luna);
    panel.entries[0].label = "line\x1b[2J\u{202e}\n".repeat(80);
    model.menu.open(panel);
    for columns in [1, 10, 25, 40, 80, 140] {
        for height in [1, 9, 12, 24, 40] {
            let rows = view::chrome(&model, columns, height);
            assert!(rows.len() < height || height == 1, "{columns}x{height}");
            assert!(
                rows.iter()
                    .all(|r| r.transient && text::width(&r.text) < columns)
            );
            assert!(
                rows.iter()
                    .all(|r| !r.text.contains(['\x1b', '\n', '\u{202e}']))
            );
        }
    }
}

#[test]
fn pending_context_preserves_the_next_draft_and_busy_help_cannot_split_a_response() {
    let (mut model, mut session) = ready();
    command(&mut model, &mut session, "/context");
    account::input(&mut model, Key::Text("next prompt".into()), &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.editor.text, "next prompt");
    assert!(model.blocks.is_empty());
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    let report = loop {
        if let Some(event) = session.poll() {
            break event;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert!(matches!(report, Event::ContextReport(_)));
    account::event(&mut model, report);
    assert_eq!(model.editor.text, "next prompt");
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.blocks.len(), 3); // Report, user prompt, streaming answer.
    command(&mut model, &mut session, "/help");
    assert_eq!(model.editor.text, "/help");
    assert_eq!(model.blocks.len(), 3);
    account::event(&mut model, Event::Text("Uninterrupted response".into()));
    assert_eq!(model.blocks.last().unwrap().text, "Uninterrupted response");
}
