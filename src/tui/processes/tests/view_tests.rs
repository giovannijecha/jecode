use super::*;

#[test]
fn finished_terminal_keeps_clean_log_and_leaves_direct_input() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let running = Process {
        id: "terminal-1".into(),
        command: "example".into(),
        status: "running",
        exit_code: None,
        error: None,
    };
    let raw = "\x1b]0;private title\x07hello\rDONE\x1b[31m\n";
    let mut output = Stream::default();
    output.pages.push_back(Page {
        text: raw.into(),
        start: 0,
        end: raw.len() as u64,
        length: raw.len() as u64,
    });
    let mut panel = Panel::new(vec![running.clone()]);
    panel.logs = Some(Logs {
        id: running.id.clone(),
        process: Some(running.clone()),
        stream: 0,
        views: [output, Stream::default()],
        interactive: true,
        input_mode: true,
    });
    app.state.processes = Some(panel);
    let finished = Process {
        status: "exited",
        exit_code: Some(0),
        ..running
    };
    assert!(app.state.processes.as_mut().unwrap().update(vec![finished]));
    let logs = app.state.processes.as_ref().unwrap().logs.as_ref().unwrap();
    assert!(!logs.input_mode);
    assert!(logs.interactive);
    let rendered = frame(&app).join("\n");
    assert!(rendered.contains("DONEo"), "{rendered}");
    assert!(!rendered.contains("[ESC]"), "{rendered}");
    assert!(!rendered.contains("i  input"), "{rendered}");
}

#[test]
fn selection_follows_process_identity_and_the_draft_survives() {
    let process = |id: &str, status| Process {
        id: id.into(),
        command: format!("command {id}"),
        status,
        exit_code: None,
        error: None,
    };
    let mut list = Panel::new(vec![process("a", "running"), process("b", "running")]);
    list.selected = 1;
    // Finished processes disappear while selection stays on the same active ID.
    assert!(list.update(vec![process("b", "running"), process("a", "exited")]));
    assert_eq!(list.selected_process().unwrap().id, "b");
    assert_eq!(list.processes.len(), 1);
    assert!(!list.update(vec![process("b", "running"), process("a", "exited")]));
    // A vanished selection clamps to the list.
    list.update(vec![process("c", "running")]);
    assert_eq!(list.selected, 0);
    list.update(vec![]);
    assert!(list.selected_process().is_none());

    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.queue.push("queued prompt").unwrap();
    app.state.editor.insert("unsent draft");
    app.open_processes("");
    assert!(panel(&app).processes.is_empty());
    assert!(
        frame(&app)
            .iter()
            .any(|line| line.contains("Nothing running"))
    );
    key(&mut app, 13, 0);
    key(&mut app, 40, 0);
    assert!(panel(&app).logs.is_none());
    app.input(Decoded::Text("x".into())).unwrap();
    key(&mut app, ESC, 0);
    assert!(app.state.processes.is_none());
    assert_eq!(app.state.editor.prompt().text, "unsent draft");
    assert!(!app.state.queue.is_empty());
    app.open_processes("process-1-2-3");
    assert!(panel(&app).logs.is_none());
}

#[test]
fn finished_error_and_narrow_states_render_within_the_panel() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let long = format!(
        "printf '{}' && {}",
        "héllo 世界 ".repeat(20),
        "x".repeat(200)
    );
    app.state.processes = Some(Panel::new(vec![
        Process {
            id: "process-running".into(),
            command: long.clone(),
            status: "running",
            exit_code: None,
            error: None,
        },
        Process {
            id: "process-failed".into(),
            command: "make test\nsecond line".into(),
            status: "exited",
            exit_code: Some(2),
            error: None,
        },
        Process {
            id: "process-unknown".into(),
            command: "true".into(),
            status: "unknown",
            exit_code: None,
            error: Some("The command started but its completion could not be confirmed".into()),
        },
    ]));
    let list = frame(&app).join("\n");
    assert!(list.contains("running"), "{list}");
    for finished in ["exited 2", "outcome unknown", "make test second line"] {
        assert!(!list.contains(finished), "{finished}\n{list}");
    }

    let panel = app.state.processes.as_mut().unwrap();
    let output = Stream {
        error: Some("Could not read saved stdout for process-unknown: missing".into()),
        ..Default::default()
    };
    panel.logs = Some(Logs {
        id: "process-unknown".into(),
        process: Some(Process {
            id: "process-unknown".into(),
            command: "true".into(),
            status: "unknown",
            exit_code: None,
            error: Some("The command started but its completion could not be confirmed".into()),
        }),
        stream: 0,
        views: [output, Stream::default()],
        interactive: false,
        input_mode: false,
    });
    let details = frame(&app).join("\n");
    assert!(
        details.contains("outcome unknown · process-unknown"),
        "{details}"
    );
    assert!(details.contains("could not be confirmed"), "{details}");
    assert!(details.contains("Could not read saved stdout"), "{details}");
    let panel = app.state.processes.as_mut().unwrap();
    let logs = panel.logs.as_mut().unwrap();
    logs.stream = 1;
    assert!(frame(&app).iter().any(|line| line.contains("No output")));

    let panel = app.state.processes.as_mut().unwrap();
    panel.selected = 0;
    let mut output = Stream::default();
    let text = format!("{}\n", "宽字符 and words ".repeat(40));
    output.pages.push_back(Page {
        end: text.len() as u64,
        length: text.len() as u64,
        text,
        start: 0,
    });
    panel.logs = Some(Logs {
        id: "process-running".into(),
        process: panel.processes.first().cloned(),
        stream: 0,
        views: [output, Stream::default()],
        interactive: false,
        input_mode: false,
    });
    for (width, height) in [(24, 12), (12, 8), (80, 30)] {
        app.state.width = width;
        app.state.height = height;
        for logs in [true, false] {
            if !logs {
                let panel = app.state.processes.as_mut().unwrap();
                panel.logs = None;
            }
            let lines = frame(&app);
            assert!(lines.len() <= height, "{width}x{height}: {lines:?}");
            for line in &lines {
                assert!(text::cells(line) <= width, "{width}x{height}: {line:?}");
            }
        }
        let panel = app.state.processes.as_mut().unwrap();
        if panel.logs.is_none() {
            panel.logs = Some(Logs {
                id: "process-running".into(),
                process: panel.processes.first().cloned(),
                stream: 1,
                views: Default::default(),
                interactive: false,
                input_mode: false,
            });
        }
    }
}
