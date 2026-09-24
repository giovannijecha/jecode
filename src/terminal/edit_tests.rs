use super::*;
use crate::{
    session::{Event, Model as Selected, edit_tests as fixture},
    terminal::{
        Key, account,
        render::Renderer,
        style::Tone,
        view::{self, Layout},
        vt,
    },
};
use std::{fs, time::Instant};

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn saturated_effect_completions_reconcile_into_the_active_tui() {
    use crate::session::outcome_backpressure_tests::{self, Case};

    for (case, expected) in [
        (Case::Edit, "Edited notes.txt"),
        (Case::Command, "Command finished"),
        (Case::TwoEdits, "Created created.txt"),
        (Case::FailedEdit, "changed since"),
    ] {
        let Some(events) = outcome_backpressure_tests::saturated_events(case) else {
            return;
        };
        let mut model = account::model(Selected::Luna, None);
        account::event(&mut model, Event::Ready);
        account::event(
            &mut model,
            Event::Guidance {
                text: "isolated effects".into(),
                new_turn: true,
            },
        );
        for event in events {
            account::event(&mut model, event);
        }
        let text = model
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("Turn stopped before"), "{text}");
        assert!(text.contains(expected), "{text}");
        if matches!(case, Case::TwoEdits) {
            assert_eq!(text.matches("Edited notes.txt").count(), 1, "{text}");
            assert_eq!(text.matches("Created created.txt").count(), 1, "{text}");
        }
        assert!(model.account.as_ref().unwrap().edit.is_none());
        assert!(model.account.as_ref().unwrap().command.is_none());
    }
}

#[test]
fn storage_failure_keeps_known_effect_and_warns_only_for_missing_outcome() {
    let preview = Preview {
        path: "notes.txt".into(),
        create: false,
        diff: "- old\n+ new\n".into(),
        added: 1,
        removed: 1,
        omitted_lines: 0,
        omitted_bytes: 0,
    };
    for delivered in [true, false] {
        let mut model = account::model(Selected::Luna, None);
        account::event(&mut model, Event::Ready);
        account::event(
            &mut model,
            Event::Guidance {
                text: "edit".into(),
                new_turn: true,
            },
        );
        account::event(
            &mut model,
            Event::EditPlanned {
                id: 1,
                preview: preview.clone(),
            },
        );
        if delivered {
            account::event(
                &mut model,
                Event::EditFinished {
                    id: 1,
                    summary: "Edited notes.txt".into(),
                    applied: true,
                    failed: false,
                },
            );
        }
        account::event(
            &mut model,
            Event::Finished(
                crate::session::End::Failed(crate::session::Failure::Storage),
                Default::default(),
            ),
        );
        let text = model
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(text.contains("Turn stopped before"), !delivered, "{text}");
        assert_eq!(text.contains("✓ Edited notes.txt"), delivered, "{text}");
        assert!(text.contains("Session could not be saved"), "{text}");
    }
}

#[test]
fn real_direct_edit_shows_one_diff_and_result_while_preserving_draft() {
    let mut run = fixture::start();
    let mut model = account::model(Selected::Luna, Some(&run.files.0));
    account::event(&mut model, Event::Ready);
    model.editor.insert("change the fixture");
    account::input(&mut model, Key::Enter, &mut run.session);
    model.editor.insert("saved draft");
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    let mut terminal = vt::Screen::new(120, 35);
    let mut saw_plan = false;
    loop {
        let event = fixture::next(&mut run.session);
        let done = matches!(event, Event::Finished(..));
        saw_plan |= matches!(event, Event::EditPlanned { .. });
        account::event(&mut model, event);
        for size in [(120, 35), (40, 18), (120, 35)] {
            terminal.resize(size.0, size.1);
            terminal.feed(&renderer.draw(layout.frame(&model, size.0, size.1), size, false));
        }
        if done {
            break;
        }
    }
    assert!(saw_plan);
    let text = terminal.text();
    assert_eq!(text.matches("Edit notes.txt").count(), 1, "{text}");
    assert_eq!(text.matches("saved draft").count(), 1, "{text}");
    assert!(!text.contains("Enter confirm"));
    assert!(!text.contains("Allow once"));
    assert_eq!(model.editor.text, "saved draft");
    assert!(model.account.as_ref().unwrap().edit.is_none());
    if !run.files.unsupported_host_filesystem(&text) {
        assert!(text.contains("Edited notes.txt"), "{text}");
        assert_eq!(
            fs::read_to_string(run.files.0.join("notes.txt")).unwrap(),
            "new\n"
        );
    }
}

#[test]
fn bounded_diff_keeps_composer_footer_and_activity_in_place() {
    let files = crate::workspace_fixture::Fixture::new();
    let workspace = crate::workspace::Workspace::open(&files.0).unwrap();
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let budget = crate::workspace::Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + std::time::Duration::from_secs(10),
    };
    let content: String = (0..401).map(|n| format!("line {n:03} abc\n")).collect();
    let change = workspace
        .prepare_create("large.html", &content, &budget)
        .unwrap();
    assert!(change.preview().omitted_lines > 0);
    let mut model = account::model(Selected::Luna, Some(&files.0));
    account::event(&mut model, Event::Ready);
    model.editor.insert("retained composer");
    planned(&mut model, 71, change.preview().clone());
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    let mut terminal = vt::Screen::new(120, 35);
    for size in [(120, 35), (40, 18), (120, 35)] {
        terminal.resize(size.0, size.1);
        let rows = layout.frame(&model, size.0, size.1);
        assert!(rows.iter().any(|r| r.text.contains("Applying file change")));
        assert!(rows.iter().any(|r| r.text.contains("retained composer")));
        assert!(!rows.iter().any(|r| r.text.contains("Enter confirm")));
        terminal.feed(&renderer.draw(rows, size, false));
    }
    let text = terminal.text();
    assert_eq!(text.matches("Create large.html").count(), 1);
    assert!(text.contains("omitted"));
    assert_eq!(model.editor.text, "retained composer");
    finished(&mut model, 71, "Created large.html".into(), true, false);
    terminal.feed(&renderer.draw(layout.frame(&model, 120, 35), (120, 35), false));
    let text = terminal.text();
    assert_eq!(text.matches("Create large.html").count(), 1);
    assert_eq!(text.matches("retained composer").count(), 1);
    assert!(model.account.as_ref().unwrap().edit.is_none());
}

#[test]
fn small_chrome_remains_bounded_during_edit() {
    let mut model = account::model(Selected::Luna, None);
    planned(
        &mut model,
        1,
        Preview {
            path: "notes.txt".into(),
            create: false,
            diff: "- old\n+ new\n".into(),
            added: 1,
            removed: 1,
            omitted_lines: 0,
            omitted_bytes: 0,
        },
    );
    for width in [2, 10, 25, 40, 80, 140] {
        for height in [2, 9, 12, 24] {
            let rows = view::chrome(&model, width, height);
            assert!(rows.len() < height);
            assert!(
                rows.iter()
                    .all(|r| r.transient && crate::terminal::text::width(&r.text) < width)
            );
        }
    }
    let rows = view::frame(&model, 80, 24);
    assert!(rows.iter().any(|row| row.tone == Tone::Removed));
    assert!(rows.iter().any(|row| row.tone == Tone::Added));
}
