use super::*;
use crate::tui::state::Item;

#[test]
fn root_and_final_notices_keep_their_cross_process_arrival_order() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    bash("first", "until test -e finish_a; do sleep 0.02; done", true),
                    bash(
                        "second",
                        "(until test -e release_b; do sleep 0.02; done) & until test -e root_b; do sleep 0.02; done; exit 7",
                        true,
                    ),
                ],
            ),
        ),
        (200, completion("Both commands started.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Start both commands.");
    wait_until(&mut app, |app| {
        app.idle() && app.state.background_processes == 2
    });
    let observer = app.process_observer.clone();
    let processes = observer.processes().unwrap();
    let first = processes
        .iter()
        .find(|process| process.command.contains("finish_a"))
        .unwrap()
        .id
        .clone();
    let second = processes
        .iter()
        .find(|process| process.command.contains("root_b"))
        .unwrap()
        .id
        .clone();
    std::fs::write(directory.path().join("finish_a"), "").unwrap();
    let started = Instant::now();
    while !observer
        .completions_since(0)
        .1
        .iter()
        .any(|done| done.id == first)
    {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "first command did not finish"
        );
        thread::sleep(Duration::from_millis(10));
    }
    std::fs::write(directory.path().join("root_b"), "").unwrap();
    while !observer
        .root_notices_since(0)
        .1
        .iter()
        .any(|root| root.id == second)
    {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "second root did not exit"
        );
        thread::sleep(Duration::from_millis(10));
    }
    app.poll_processes();
    let position = |id: &str| {
        app.state
            .items
            .iter()
            .position(|item| matches!(item, Item::Tool { id: current, .. } if current == id))
            .unwrap()
    };
    assert!(position(&format!("process:{first}")) < position(&format!("process-root:{second}")));
    assert_eq!(fixture.finish().len(), 2);
}

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
