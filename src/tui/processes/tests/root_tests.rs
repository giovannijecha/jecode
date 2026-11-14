use super::*;
use crate::tui::state::Item;

#[test]
fn a_root_notice_is_visible_saved_and_resumed_once_before_the_tree_ends() {
    let home = Directory::new();
    let directory = Directory::new();
    let command = "(until test -e release; do sleep 0.02; done) & until test -e root_exit; do sleep 0.02; done; exit 7";
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![bash("start", command, true)])),
        (200, completion("Waiting for the root.", vec![])),
        (
            200,
            completion("The root exited; its child is still running.", vec![]),
        ),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, tests::config(&directory), None);
    app.state.width = 80;
    app.state.height = 30;
    let mut app = tests::full_app(app);
    submit(&mut app, "Build the fixture.");
    wait_until(&mut app, |app| {
        app.idle() && app.state.background_processes == 1
    });
    std::fs::write(directory.path().join("root_exit"), "").unwrap();
    let root_rows = |app: &App| {
        app.state
            .items
            .iter()
            .filter(|item| matches!(item, Item::Tool { id, .. } if id.starts_with("process-root:")))
            .count()
    };
    wait_until(&mut app, |app| root_rows(app) == 1 && app.idle());
    for _ in 0..8 {
        app.poll().unwrap();
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(root_rows(&app), 1);
    assert_eq!(app.state.background_processes, 1);
    assert!(
        !app.state
            .items
            .iter()
            .any(|item| { matches!(item, Item::Tool { id, .. } if id.starts_with("process:")) })
    );
    let records = app.persistence.as_ref().unwrap().snapshot().records();
    assert_eq!(
        records
            .iter()
            .filter(|record| {
                matches!(record, crate::sessions::Record::ProcessRootExited { .. })
            })
            .count(),
        1
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[2]
            .body
            .encode()
            .contains("Process roots that exited")
    );
}
