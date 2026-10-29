use super::*;
use crate::tui::line::Line;

fn press(app: &mut App, code: u16, modifiers: u8) -> bool {
    app.input(Decoded::Key(terminal::Key {
        code,
        modifiers,
        character: 0,
    }))
    .unwrap()
}

#[test]
fn ctrl_q_quits_at_once_when_idle_and_ctrl_c_never_quits() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    assert!(!press(&mut app, 67, 4));
    assert!(app.state.quitting.is_none());
    assert!(press(&mut app, 81, 4));
}

#[test]
fn ctrl_q_asks_while_a_turn_runs_and_only_enter_or_ctrl_q_quits() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Done.", vec![]))]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Work for a while.");
    assert!(app.worker.is_some());
    assert!(!press(&mut app, 81, 4));
    assert_eq!(app.state.quitting.as_deref(), Some(&[][..]));
    let frame = crate::tui::view::frame(&app.state, "fixture/model", "fixture");
    let dock: Vec<_> = frame.live[frame.composer..]
        .iter()
        .map(Line::plain)
        .collect();
    assert!(
        dock.iter()
            .any(|row| row.contains("─ Quit · work is running"))
    );
    assert!(dock.iter().any(|row| row.starts_with("  Current turn  ")));
    assert!(
        dock.iter()
            .any(|row| row.contains("Stop it and quit?") && row.contains(" Enter  quit"))
    );
    // Typing and Ctrl+C leave the draft and the turn alone; reading back
    // keeps the question open.
    app.input(Decoded::Text("x".into())).unwrap();
    assert!(app.state.editor.text.is_empty());
    assert!(!press(&mut app, 35, 1));
    assert!(app.input(Decoded::Scroll(-3)).is_ok_and(|quit| !quit));
    assert!(app.state.quitting.is_some());
    assert!(!press(&mut app, 67, 4));
    assert!(app.state.quitting.is_none());
    assert!(app.worker.is_some());
    assert!(!press(&mut app, 81, 4));
    assert!(!press(&mut app, 27, 0));
    assert!(app.state.quitting.is_none());
    assert!(!press(&mut app, 81, 4));
    assert!(press(&mut app, 13, 0));
    assert!(app.state.quitting.is_none());
    assert!(
        app.state
            .activity
            .as_ref()
            .is_some_and(|turn| turn.stopping)
    );
}

#[test]
fn ctrl_q_asks_while_a_collaborator_works_and_names_only_working_ones() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let record = |number: usize, title: &str, state: &str| {
        Value::object([
            ("id", Value::string(format!("a{number}"))),
            ("number", Value::number(number)),
            ("title", Value::string(title)),
            ("state", Value::string(state)),
        ])
    };
    app.state.collaboration = vec![
        record(1, "Check CSV encoding", "running"),
        record(2, "Review browser download", "returned"),
    ];
    assert!(!press(&mut app, 81, 4));
    assert_eq!(
        app.state.quitting.as_deref(),
        Some(
            &[(
                "Check CSV encoding".to_owned(),
                "agent 1 running".to_owned()
            )][..]
        )
    );
    let frame = crate::tui::view::frame(&app.state, "fixture/model", "fixture");
    let dock: Vec<_> = frame.live[frame.composer..]
        .iter()
        .map(Line::plain)
        .collect();
    assert!(
        dock.iter()
            .any(|row| row.starts_with("  Check CSV encoding  agent 1 running"))
    );
    assert!(
        !dock
            .iter()
            .any(|row| row.contains("Review browser download"))
    );
    assert!(press(&mut app, 13, 0));
    app.state.collaboration[0] = record(1, "Check CSV encoding", "returned");
    assert!(press(&mut app, 81, 4));
}
