use super::*;
use crate::{
    session::{Event, Model as Selected, edit_tests as fixture},
    terminal::{
        account,
        render::Renderer,
        style::Tone,
        view::{self, Layout},
        vt,
    },
};
use std::{fs, time::Instant};

#[test]
fn actual_session_approval_uses_the_diff_surface_and_preserves_draft_through_resize() {
    for allow in [false, true] {
        let mut run = fixture::start();
        let mut model = account::model(Selected::Luna, Some(&run.files.0));
        account::event(&mut model, Event::Ready);
        model.editor.insert("change the fixture");
        account::input(&mut model, Key::Enter, &mut run.session);
        model.editor.insert("saved draft");
        account::event(&mut model, fixture::next(&mut run.session));
        assert!(model.account.as_ref().unwrap().approval.is_some());
        assert!(
            !model
                .account
                .as_ref()
                .unwrap()
                .approval
                .as_ref()
                .unwrap()
                .allow
        );
        // No invisible approval, and paste cannot synthesize key decisions.
        account::input(&mut model, Key::Right, &mut run.session);
        account::input(&mut model, Key::Enter, &mut run.session);
        assert!(
            !model
                .account
                .as_ref()
                .unwrap()
                .approval
                .as_ref()
                .unwrap()
                .submitted
        );
        let mut decoder = crate::terminal::input::Decoder::default();
        for key in decoder.push(b"\x1b[200~\x1b[C\ryes\x1b[201~", Instant::now()) {
            account::input(&mut model, key, &mut run.session);
        }
        assert_eq!(model.editor.text, "saved draft");
        let mut layout = Layout::default();
        let mut renderer = Renderer::default();
        let mut terminal = vt::Screen::new(120, 35);
        for size in [(120, 35), (40, 18), (120, 35)] {
            terminal.resize(size.0, size.1);
            let rows = layout.frame(&model, size.0, size.1);
            assert!(
                rows.iter()
                    .any(|r| r.tone == Tone::Removed && r.text.contains("old"))
            );
            assert!(
                rows.iter()
                    .any(|r| r.tone == Tone::Added && r.text.contains("new"))
            );
            terminal.feed(&renderer.draw(rows, size, false));
            displayed(&mut model, size, true);
        }
        assert_eq!(
            fs::read_to_string(run.files.0.join("notes.txt")).unwrap(),
            "old\n"
        );
        if allow {
            account::input(&mut model, Key::Right, &mut run.session);
        }
        account::input(&mut model, Key::Enter, &mut run.session);
        loop {
            let event = fixture::next(&mut run.session);
            let another = matches!(event, Event::EditProposed { .. });
            let finished = matches!(event, Event::Finished(..));
            account::event(&mut model, event);
            terminal.feed(&renderer.draw(layout.frame(&model, 120, 35), (120, 35), false));
            displayed(&mut model, (120, 35), true);
            if another {
                account::input(&mut model, Key::Escape, &mut run.session);
            }
            if finished {
                break;
            }
        }
        let text = terminal.text();
        assert_eq!(text.matches("Edit notes.txt").count(), 1, "{text}");
        assert_eq!(text.matches("saved draft").count(), 1, "{text}");
        assert!(!text.contains("Enter confirm"), "approval archived: {text}");
        assert_eq!(model.editor.text, "saved draft");
        assert!(model.account.as_ref().unwrap().approval.is_none());
        if allow && !run.files.unsupported_host_filesystem(&text) {
            assert!(text.contains("Edited notes.txt"), "{text}");
            assert_eq!(
                fs::read_to_string(run.files.0.join("notes.txt")).unwrap(),
                "new\n"
            );
        } else {
            assert_eq!(
                fs::read_to_string(run.files.0.join("notes.txt")).unwrap(),
                "old\n"
            );
        }
    }
}

#[test]
fn hidden_approval_cannot_be_confirmed_and_small_chrome_stays_bounded() {
    let mut run = fixture::start();
    let mut model = account::model(Selected::Luna, Some(&run.files.0));
    account::event(&mut model, Event::Ready);
    model.editor.insert("change");
    account::input(&mut model, Key::Enter, &mut run.session);
    account::event(&mut model, fixture::next(&mut run.session));
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
    displayed(&mut model, (120, 35), true);
    account::input(&mut model, Key::Right, &mut run.session);
    displayed(&mut model, (10, 4), false);
    account::input(&mut model, Key::Enter, &mut run.session);
    assert!(
        !model
            .account
            .as_ref()
            .unwrap()
            .approval
            .as_ref()
            .unwrap()
            .submitted
    );
    account::input(&mut model, Key::Interrupt, &mut run.session);
    loop {
        let event = fixture::next(&mut run.session);
        let finished = matches!(event, Event::Finished(..));
        account::event(&mut model, event);
        if finished {
            break;
        }
    }
    assert_eq!(
        fs::read_to_string(run.files.0.join("notes.txt")).unwrap(),
        "old\n"
    );
    assert_eq!(fs::read_dir(&run.files.0).unwrap().count(), 1);
}
