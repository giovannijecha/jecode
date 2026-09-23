use super::{
    Key, account,
    model::Model,
    render::Renderer,
    view::{self, Layout},
    vt,
};
use crate::session::{self, End, Event, Metrics};

fn draw(
    model: &Model,
    layout: &mut Layout,
    renderer: &mut Renderer,
    terminal: &mut vt::Screen,
    size: (usize, usize),
) {
    terminal.resize(size.0, size.1);
    let output = renderer.draw(layout.frame(model, size.0, size.1), size, false);
    assert!(!output.contains("\x1b[2J") && !output.contains("\x1b[3J"));
    terminal.feed(&output);
}

fn chrome_is_singular(shown: &str, draft: &str) {
    assert_eq!(shown.matches(draft).count(), 1, "{shown}");
    assert_eq!(shown.matches("Conversation only").count(), 1, "{shown}");
    assert_eq!(
        shown
            .lines()
            .filter(|line| line.contains('\u{2500}'))
            .count(),
        2,
        "{shown}"
    );
}

fn numbered_response_is_intact(shown: &str, count: usize, correction: &str) {
    assert_eq!(shown.matches("first").count(), 1, "count={count}: {shown}");
    let compact: String = shown.chars().filter(|ch| !ch.is_whitespace()).collect();
    let correction: String = correction
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect();
    assert_eq!(
        compact.matches(&correction).count(),
        1,
        "count={count}: {shown}"
    );
    assert_eq!(
        shown.matches("Final response correction").count(),
        1,
        "count={count}: {shown}"
    );
    for n in 0..count {
        assert_eq!(
            shown.matches(&format!("line-{n:03}")).count(),
            1,
            "count={count}: {shown}"
        );
    }
    chrome_is_singular(shown, "draft-kept");
}

#[test]
fn reconciliation_preserves_scrollback_before_and_after_resize_and_next_turn() {
    for count in [3, 45] {
        let mut model = account::model(session::Model::Luna, None);
        account::event(&mut model, Event::Ready);
        let mut session = session::tests::ready_fixture();
        account::input(
            &mut model,
            Key::Text("Inspect fixture".into()),
            &mut session,
        );
        account::input(&mut model, Key::Enter, &mut session);
        model.editor.insert("draft-kept");
        let mut layout = Layout::default();
        let mut renderer = Renderer::default();
        let mut terminal = vt::Screen::new(80, 24);
        let tail: String = (0..count).map(|n| format!("\nline-{n:03}")).collect();
        account::event(&mut model, Event::Text(format!("first{tail}")));
        draw(&model, &mut layout, &mut renderer, &mut terminal, (80, 24));
        draw(&model, &mut layout, &mut renderer, &mut terminal, (63, 27));
        let addition = if count == 3 { " suffix" } else { " suffix\n" };
        account::event(
            &mut model,
            Event::TextReconciled(format!("first{addition}{tail}")),
        );
        draw(&model, &mut layout, &mut renderer, &mut terminal, (63, 27));
        let correction = if count == 3 {
            "insert \" suffix\""
        } else {
            "insert \" suffix\" followed by a line break"
        };
        numbered_response_is_intact(&terminal.text(), count, correction);
        account::event(
            &mut model,
            Event::Finished(End::Complete, Metrics::default()),
        );
        draw(&model, &mut layout, &mut renderer, &mut terminal, (63, 27));
        draw(&model, &mut layout, &mut renderer, &mut terminal, (96, 30));
        numbered_response_is_intact(&terminal.text(), count, correction);

        account::event(
            &mut model,
            Event::Guidance {
                text: "Next turn".into(),
                new_turn: true,
            },
        );
        account::event(&mut model, Event::Text("Next answer.".into()));
        account::event(
            &mut model,
            Event::Finished(End::Complete, Metrics::default()),
        );
        draw(&model, &mut layout, &mut renderer, &mut terminal, (96, 30));
        let shown = terminal.text();
        assert_eq!(shown.matches("Next turn").count(), 1, "{shown}");
        assert_eq!(shown.matches("Next answer.").count(), 1, "{shown}");
        numbered_response_is_intact(&shown, count, correction);
    }
}

#[test]
fn unindexed_repeated_messages_show_a_paragraph_correction_without_erasing_draft_or_menu() {
    let mut model = account::model(session::Model::Luna, None);
    account::event(&mut model, Event::Ready);
    let mut session = session::tests::boundaries_fixture();
    account::input(
        &mut model,
        Key::Text("Inspect repeated messages".into()),
        &mut session,
    );
    account::input(&mut model, Key::Enter, &mut session);
    model.editor.insert("/he");
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    let mut terminal = vt::Screen::new(80, 24);
    draw(&model, &mut layout, &mut renderer, &mut terminal, (80, 24));
    let mut reconciled = false;
    loop {
        let event = session::tests::next(&mut session);
        let just_reconciled = matches!(event, Event::TextReconciled(_));
        if let Event::TextReconciled(text) = &event {
            assert_eq!(text, "Same\n\nSame");
            reconciled = true;
        }
        let finished = matches!(event, Event::Finished(End::Complete, _));
        account::event(&mut model, event);
        draw(&model, &mut layout, &mut renderer, &mut terminal, (80, 24));
        if just_reconciled {
            let shown = terminal.text();
            assert_eq!(shown.matches("SameSame").count(), 1, "{shown}");
            assert_eq!(
                shown.matches("insert a paragraph break").count(),
                1,
                "{shown}"
            );
            chrome_is_singular(&shown, "/he|");
        }
        if finished {
            break;
        }
    }
    assert!(reconciled);
    for size in [(55, 25), (100, 30), (80, 24)] {
        draw(&model, &mut layout, &mut renderer, &mut terminal, size);
        let shown = terminal.text();
        assert_eq!(shown.matches("SameSame").count(), 1, "{size:?}: {shown}");
        assert_eq!(
            shown.matches("insert a paragraph break").count(),
            1,
            "{size:?}: {shown}"
        );
        assert_eq!(
            shown.matches("Final response correction").count(),
            1,
            "{size:?}: {shown}"
        );
        assert_eq!(shown.matches("/help").count(), 1, "{size:?}: {shown}");
        chrome_is_singular(&shown, "/he|");
    }
    let frame = view::chrome(&model, 80, 24);
    let upper = frame
        .iter()
        .position(|row| row.text.starts_with('\u{2500}'))
        .unwrap();
    let lower = frame
        .iter()
        .rposition(|row| row.text.starts_with('\u{2500}'))
        .unwrap();
    assert!(
        frame[..upper]
            .iter()
            .any(|row| row.text.contains("Complete"))
    );
    assert!(
        frame[upper..]
            .iter()
            .all(|row| !row.text.contains("Complete"))
    );
    assert!(
        frame[upper + 1..lower]
            .iter()
            .any(|row| row.text.contains("/help"))
    );
    assert_eq!(frame.len() - lower - 1, 1);
}
