use super::*;
use crate::tools::processes::Window;

#[test]
fn the_list_excludes_finished_commands_and_keeps_an_open_final_log() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    bash(
                        "server",
                        "printf 'ready\\n'; until [ -e finish ]; do sleep 0.02; done; printf 'final\\n'; exit 7",
                        true,
                    ),
                    bash("short", "printf short", true),
                    bash("foreground", "printf foreground", false),
                ],
            ),
        ),
        (200, completion("Started.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Start.");
    wait_until(&mut app, |app| app.worker.is_none());
    wait_until(&mut app, |app| {
        app.process_observer.processes().is_some_and(|processes| {
            processes.len() == 2 && processes.iter().filter(|p| p.active()).count() == 1
        })
    });
    app.open_processes("");
    assert_eq!(panel(&app).processes.len(), 1);
    let id = panel(&app).processes[0].id.clone();
    key(&mut app, 13, 0);
    create(&directory, "finish");
    wait_until(&mut app, |app| {
        app.refresh_processes(true);
        panel(app).processes.is_empty()
            && panel(app)
                .selected_process()
                .is_some_and(|p| p.label() == "exited 7")
    });
    assert_eq!(logs(&app).id, id);
    assert!(stream(&app).text().ends_with("ready\nfinal\n"));
    assert!(frame(&app).iter().any(|line| line.trim() == "final"));
    // The long command title takes the bottom rule first; with room the
    // final status shows as its aside.
    app.state.width = 160;
    let rule = frame(&app)
        .into_iter()
        .find(|line| line.starts_with("─ Processes"));
    assert!(rule.is_some_and(|rule| rule.contains(&format!("exited 7 · {id} ─"))));
    app.state.width = 80;
    key(&mut app, ESC, 0);
    assert!(panel(&app).logs.is_none());
    assert!(frame(&app).join("\n").contains("Nothing running"));
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn stopping_from_a_paused_log_preserves_the_draft_and_the_active_turn() {
    let directory = Directory::new();
    let fixture = running_turn();
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Start.");
    let observer = app.process_observer.clone();
    wait_until(&mut app, |_| {
        observer.processes().is_some_and(|processes| {
            processes.first().is_some_and(|process| {
                observer
                    .read(&process.id, 0, Window::Tail)
                    .is_ok_and(|page| page.text.ends_with("29999\n30000\n"))
            })
        })
    });
    let id = observer.processes().unwrap()[0].id.clone();
    let history = app.state.history.snapshot();
    app.state.editor.insert("keep this draft");
    app.state.queue.push("keep this queued prompt").unwrap();
    app.state.queue.pause();
    app.open_processes(&id);
    key(&mut app, UP, 0);
    let anchor = stream(&app).anchor;
    assert!(!stream(&app).follow);

    // Both cancellation keys leave the same paused log and live process.
    for cancel in [(ESC, 0), (67, 4)] {
        key(&mut app, 68, 4);
        assert_eq!(panel(&app).confirm_stop.as_ref().unwrap().id, id);
        app.input(Decoded::Scroll(-10)).unwrap();
        key(&mut app, cancel.0, cancel.1);
        assert!(panel(&app).confirm_stop.is_none());
        assert_eq!(logs(&app).id, id);
        assert_eq!(stream(&app).anchor, anchor);
        assert!(observer.processes().unwrap()[0].active());
    }
    key(&mut app, 68, 4);
    key(&mut app, 13, 0);
    wait_until(&mut app, |app| {
        app.refresh_processes(true);
        panel(app)
            .selected_process()
            .is_some_and(|p| p.status == "stopped")
    });
    assert!(panel(&app).processes.is_empty());
    assert!(!stream(&app).follow);
    assert_eq!(stream(&app).anchor, anchor);
    assert!(app.worker.is_some() && app.agent.is_none());
    assert!(!app.state.activity.as_ref().unwrap().stopping);
    assert_eq!(app.state.editor.prompt().text, "keep this draft");
    assert_eq!(app.state.history.snapshot(), history);
    assert_eq!(app.state.queue.len(), 1);

    create(&directory, "turn");
    wait_until(&mut app, |app| app.worker.is_none());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn a_finished_confirmation_never_stops_the_next_selected_process() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    bash(
                        "first",
                        "until [ -e finish-first ]; do sleep 0.02; done",
                        true,
                    ),
                    bash(
                        "second",
                        "until [ -e finish-second ]; do sleep 0.02; done",
                        true,
                    ),
                ],
            ),
        ),
        (200, completion("Started.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Start.");
    wait_until(&mut app, |app| app.worker.is_none());
    app.open_processes("");
    assert_eq!(panel(&app).processes.len(), 2);
    let first = panel(&app).processes[0].id.clone();
    let second = panel(&app).processes[1].id.clone();
    key(&mut app, 68, 4);
    create(&directory, "finish-first");
    wait_until(&mut app, |app| {
        app.refresh_processes(true);
        panel(app)
            .confirm_stop
            .as_ref()
            .is_some_and(|p| !p.active())
    });
    assert_eq!(panel(&app).confirm_stop.as_ref().unwrap().id, first);
    assert_eq!(panel(&app).selected_process().unwrap().id, second);
    key(&mut app, 13, 0);
    assert!(panel(&app).confirm_stop.is_none());
    assert!(panel(&app).processes[0].active());

    // The explicit command uses the same local path, with no new model call.
    key(&mut app, ESC, 0);
    submit(&mut app, &format!("/terminal stop {second}"));
    wait_until(&mut app, |app| {
        app.refresh_processes(true);
        panel(app).processes.is_empty()
    });
    assert_eq!(fixture.finish().len(), 2);
}
