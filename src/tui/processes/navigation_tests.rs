use super::*;
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    json::Value,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
    tui::{Decoded, terminal, tests, view},
};
use std::{
    thread,
    time::{Duration, Instant},
};

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    let mut app = App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        tests::config(directory),
        None,
    );
    app.state.width = 80;
    app.state.height = 30;
    tests::full_app(app)
}

fn open_output(app: &mut App, output: &str) {
    let mut stream = Stream {
        follow: false,
        ..Default::default()
    };
    stream.pages.push_back(Page {
        text: output.into(),
        start: 0,
        end: output.len() as u64,
        length: output.len() as u64,
    });
    let process = Process {
        id: "process-example".into(),
        command: "example".into(),
        status: "exited",
        exit_code: Some(0),
        error: None,
    };
    let mut panel = Panel::new(vec![]);
    panel.logs = Some(Logs {
        id: "process-example".into(),
        process: Some(process),
        stream: 0,
        views: [stream, Stream::default()],
        interactive: false,
        input_mode: false,
    });
    app.state.processes = Some(panel);
}

#[test]
fn direct_escape_returns_to_log_navigation_without_touching_the_draft() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("unsent draft");
    let history = app.state.history.snapshot();
    open_output(&mut app, "output");
    app.state
        .processes
        .as_mut()
        .unwrap()
        .logs
        .as_mut()
        .unwrap()
        .input_mode = true;
    app.input(Decoded::Key(terminal::Key {
        code: 221,
        modifiers: 4,
        character: 29,
    }))
    .unwrap();
    assert!(
        !app.state
            .processes
            .as_ref()
            .unwrap()
            .logs
            .as_ref()
            .unwrap()
            .input_mode
    );
    assert_eq!(app.state.editor.prompt().text, "unsent draft");
    assert_eq!(app.state.history.snapshot(), history);
}

fn frame(app: &App) -> Vec<String> {
    view::composer_frame(&app.state, "fixture/model", "fixture directory")
        .live
        .iter()
        .map(|line| line.plain())
        .collect()
}

fn submit(app: &mut App, prompt: &str) {
    app.input(Decoded::Text(prompt.into())).unwrap();
    app.input(Decoded::Key(terminal::Key {
        code: 13,
        modifiers: 0,
        character: 0,
    }))
    .unwrap();
}

fn wait_until(app: &mut App, mut done: impl FnMut(&mut App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(app) {
        app.poll().unwrap();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn page_down_advances_by_the_visible_log_rows() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    open_output(
        &mut app,
        &(0..100)
            .map(|index| format!("{index:03}\n"))
            .collect::<String>(),
    );
    let visible = view::process_log_rows(&app.state).1;
    app.process_key(terminal::Key {
        code: 34,
        modifiers: 0,
        character: 0,
    })
    .unwrap();
    assert_eq!(
        app.state
            .processes
            .as_ref()
            .unwrap()
            .logs
            .as_ref()
            .unwrap()
            .views[0]
            .offset,
        visible
    );
}

#[test]
fn one_column_log_rows_keep_each_ascii_character() {
    assert_eq!(view::processes::row_starts("ABCD", 1).len(), 4);
}

#[test]
fn short_panel_keeps_a_log_row_visible() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.height = 4;
    open_output(&mut app, "marker\n");
    assert!(frame(&app).iter().any(|line| line.contains("marker")));
}

#[test]
fn short_log_arrows_reach_middle_rows_before_following() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    open_output(&mut app, "row-0\nrow-1\nrow-2\nrow-3");
    app.state.height = 3;
    assert!(frame(&app).iter().any(|line| line.contains("row-0")));
    for number in 1..=2 {
        app.process_key(terminal::Key {
            code: 40,
            modifiers: 0,
            character: 0,
        })
        .unwrap();
        let rows = frame(&app);
        assert!(
            rows.iter()
                .any(|line| line.contains(&format!("row-{number}"))),
            "{rows:?}"
        );
    }
}

#[test]
fn resizing_paused_output_preserves_the_reading_position() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let output = (0..1000)
        .map(|index| format!("{index:04}"))
        .collect::<String>();
    open_output(&mut app, &output);
    app.scroll_logs(20);
    assert_eq!(
        app.state
            .processes
            .as_ref()
            .unwrap()
            .logs
            .as_ref()
            .unwrap()
            .views[0]
            .offset,
        20
    );
    app.state.width = 40;
    assert!(frame(&app).join("\n").contains("0380"));
}

#[test]
fn prepending_a_page_uses_rows_of_the_joined_output() {
    let directory = Directory::new();
    let command = "seq 0 30000 | tr -d '\\n'";
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "launch",
                    "bash",
                    Value::object([
                        ("command", Value::string(command)),
                        ("wait_ms", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (200, completion("Started.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Start.");
    wait_until(&mut app, |app| app.worker.is_none());
    wait_until(&mut app, |app| {
        app.process_observer.processes().is_some_and(|list| {
            list.first()
                .is_some_and(|process| process.label() == "exited 0")
        })
    });
    let id = app.process_observer.processes().unwrap()[0].id.clone();
    app.open_processes(&id);
    let old_start = app
        .state
        .processes
        .as_ref()
        .unwrap()
        .logs
        .as_ref()
        .unwrap()
        .views[0]
        .start();
    assert!(old_start > 0);
    {
        let stream = &mut app
            .state
            .processes
            .as_mut()
            .unwrap()
            .logs
            .as_mut()
            .unwrap()
            .views[0];
        stream.follow = false;
        stream.offset = 0;
        stream.anchor = 0;
    }
    app.scroll_logs(-1);
    let stream = &app
        .state
        .processes
        .as_ref()
        .unwrap()
        .logs
        .as_ref()
        .unwrap()
        .views[0];
    let width = view::process_width(&app.state);
    let expected = ((old_start - stream.start()) as usize / width).saturating_sub(1);
    assert_eq!(stream.offset, expected);
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn following_a_partial_utf8_character_does_not_duplicate_saved_text() {
    let directory = Directory::new();
    let command = "printf 'A\\342'; touch first-byte; until [ -e rest ]; do sleep 0.02; done; printf '\\202\\254'; until [ -e stop ]; do sleep 0.02; done";
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "launch",
                    "bash",
                    Value::object([
                        ("command", Value::string(command)),
                        ("wait_ms", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (200, completion("Started.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Start.");
    wait_until(&mut app, |app| app.worker.is_none());
    wait_until(&mut app, |_| directory.path().join("first-byte").exists());
    let observer = app.process_observer.clone();
    let id = observer.processes().unwrap()[0].id.clone();
    wait_until(&mut app, |_| {
        observer
            .read(&id, 0, Window::Tail)
            .is_ok_and(|page| page.text == "A" && page.length == 2 && page.end == 1)
    });
    app.open_processes(&id);
    for _ in 0..5 {
        assert!(!app.refresh_processes(true));
        let stream = &app
            .state
            .processes
            .as_ref()
            .unwrap()
            .logs
            .as_ref()
            .unwrap()
            .views[0];
        assert_eq!(stream.text(), "A");
        assert_eq!(stream.pages.len(), 1);
    }
    std::fs::write(directory.path().join("rest"), "").unwrap();
    wait_until(&mut app, |_| {
        observer
            .read(&id, 0, Window::Tail)
            .is_ok_and(|page| page.text == "A€")
    });
    for _ in 0..5 {
        app.refresh_processes(true);
        assert_eq!(
            app.state
                .processes
                .as_ref()
                .unwrap()
                .logs
                .as_ref()
                .unwrap()
                .views[0]
                .text(),
            "A€"
        );
    }
    std::fs::write(directory.path().join("stop"), "").unwrap();
    assert_eq!(fixture.finish().len(), 2);
}
