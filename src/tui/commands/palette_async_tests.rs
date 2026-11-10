use super::*;

#[test]
fn a_withdrawn_question_leaves_esc_with_the_composer_after_one_poll() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    let before = app.state.editor.clone();
    let request = question();
    assert!(app.question_observer.begin(request.clone()).unwrap());
    app.refresh_question();
    key(&mut app, ESC, 0);
    key(&mut app, CTRL_G, CTRL);
    text(&mut app, "/ask");
    key(&mut app, ENTER, 0);
    assert!(app.state.ask_panel.is_some() && app.from_commands);
    app.question_observer.complete(&request.id);
    app.poll().unwrap();
    assert!(app.state.ask_panel.is_none());
    key(&mut app, ESC, 0);
    assert!(!commands_open(&app));
    assert_eq!(app.state.editor, before);
}
