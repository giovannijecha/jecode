use super::*;

fn observer(tools: &Tools) -> Observer {
    tools.observe_processes(Redactor::new("fixture-secret-value".into()))
}

fn listed(observer: &Observer, id: &str) -> Process {
    observer
        .processes()
        .unwrap()
        .into_iter()
        .find(|process| process.id == id)
        .unwrap()
}

#[test]
fn viewing_output_leaves_the_models_next_poll_unchanged() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let observer = observer(&tools);
    let (id, first) = start(
        &tools,
        &format!(
            "{}; printf 'one\\n'; printf 'err\\n' >&2; {}; printf 'two\\n'",
            gate("a"),
            gate("b")
        ),
    );
    assert_eq!(text(&first, "stdout"), "");
    release(&directory, "a");
    wait_for(|| {
        observer.read(&id, 0, Window::Tail).unwrap().text == "one\n"
            && observer.read(&id, 1, Window::Tail).unwrap().text == "err\n"
    });
    // Every window shape reads independently of the poll cursors.
    for window in [Window::Tail, Window::After(0), Window::Before(4)] {
        let page = observer.read(&id, 0, window).unwrap();
        assert_eq!((page.text.as_str(), page.start, page.end), ("one\n", 0, 4));
    }
    let poll = process(&tools, "poll", &id, None);
    assert_eq!(text(&poll, "stdout"), "one\n");
    assert_eq!(text(&poll, "stderr"), "err\n");
    assert_eq!(number(&poll, "stdout_offset"), 0);
    release(&directory, "b");
    wait_for(|| listed(&observer, &id).status == "exited");
    assert_eq!(
        observer.read(&id, 0, Window::Tail).unwrap().text,
        "one\ntwo\n"
    );
    let last = process(&tools, "poll", &id, None);
    assert_eq!(text(&last, "stdout"), "two\n");
    assert_eq!(number(&last, "stdout_offset"), 4);
    assert_eq!(text(&last, "stderr"), "");
}

#[test]
fn processes_list_active_first_with_masked_commands_and_exit_codes() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::new("fixture-secret-value".into()));
    let observer = observer(&tools);
    let (finished, _) = start(&tools, "printf 'fixture-secret-value\\n'; exit 3");
    wait_for(|| listed(&observer, &finished).status == "exited");
    let (running, _) = start(&tools, &gate("go"));
    let processes = observer.processes().unwrap();
    assert_eq!(processes[0].id, running);
    assert!(processes[0].active());
    assert_eq!(processes[0].label(), "running");
    assert_eq!(processes[1].id, finished);
    assert_eq!(processes[1].label(), "exited 3");
    assert_eq!(processes[1].exit_code, Some(3));
    assert!(
        processes[1].command.contains("[redacted]")
            && !processes[1].command.contains("fixture-secret-value"),
        "{:?}",
        processes[1].command
    );
    let page = observer.read(&finished, 0, Window::Tail).unwrap();
    assert_eq!(page.text, "[redacted]\n");
    release(&directory, "go");
}

#[test]
fn windows_are_bounded_and_respect_utf8_boundaries() {
    let directory = Directory::new();
    let path = directory.path().join("stream");
    let bytes = "é".repeat(OUTPUT_LIMIT).into_bytes();
    std::fs::write(&path, &bytes).unwrap();
    let length = bytes.len() as u64;
    let tail = read_window(&path, Window::Tail, true, &Redactor::empty()).unwrap();
    assert_eq!((tail.end, tail.length), (length, length));
    assert_eq!(tail.end - tail.start, OUTPUT_LIMIT as u64);
    let older = read_window(&path, Window::Before(tail.start), true, &Redactor::empty()).unwrap();
    assert_eq!(older.end, tail.start);
    let first = read_window(&path, Window::After(0), true, &Redactor::empty()).unwrap();
    assert_eq!((first.start, first.end), (0, OUTPUT_LIMIT as u64));
    // An odd cut lands inside a character: both edges move to boundaries.
    let cut = read_window(&path, Window::Before(length - 1), true, &Redactor::empty()).unwrap();
    assert_eq!(cut.end, length - 2);
    assert_eq!(cut.start % 2, 0);
    for page in [tail, older, first, cut] {
        assert!(page.text.chars().all(|character| character == 'é'));
    }
    // A live stream holds back a partial character for a later read.
    std::fs::write(&path, &"aé".as_bytes()[..2]).unwrap();
    let live = read_window(&path, Window::Tail, false, &Redactor::empty()).unwrap();
    assert_eq!((live.text.as_str(), live.end, live.length), ("a", 1, 2));
}

#[test]
fn newly_configured_key_stays_masked_across_saved_stream_windows() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let finished = bash(&tools, "true", &[("wait_ms", Value::number(10_000))]);
    let id = text(&finished, "process_id");
    let path = tools
        .outputs
        .resolve(&text(&finished, "stdout_file"))
        .unwrap();
    // This stream was created before the key was configured, so it contains
    // the original bytes. The viewer must mask even partial window overlap.
    let key = "Q7vL4pM9sT2zB5nK8rW1yD6hF3cJ0u";
    let mut saved = vec![b'x'; OUTPUT_LIMIT * 2];
    saved[OUTPUT_LIMIT - 5..OUTPUT_LIMIT - 5 + key.len()].copy_from_slice(key.as_bytes());
    saved[OUTPUT_LIMIT - 5 + key.len()..].fill(b'y');
    std::fs::write(path, saved).unwrap();
    let observer = tools.observe_processes(Redactor::new(key.into()));
    let tail = observer.read(&id, 0, Window::Tail).unwrap();
    assert_eq!(
        (tail.start, tail.end),
        (OUTPUT_LIMIT as u64, (OUTPUT_LIMIT * 2) as u64)
    );
    let before = observer
        .read(&id, 0, Window::Before(OUTPUT_LIMIT as u64 + 5))
        .unwrap();
    let after = observer.read(&id, 0, Window::After(before.end)).unwrap();
    assert_eq!(before.end, after.start);
    let forward = observer.read(&id, 0, Window::After(10)).unwrap();
    let forward_next = observer.read(&id, 0, Window::After(forward.end)).unwrap();
    assert_eq!(forward.end, forward_next.start);
    for page in [&tail, &before, &after, &forward, &forward_next] {
        let visible = page.text.replace("[redacted]", "");
        assert!(
            visible.bytes().all(|byte| matches!(byte, b'x' | b'y')),
            "window {}..{} exposed part of the key: {:?}",
            page.start,
            page.end,
            &visible[..visible.len().min(40)]
        );
    }
    assert_eq!(
        [before.text, after.text]
            .concat()
            .matches("[redacted]")
            .count(),
        1
    );
    assert_eq!(
        [forward.text, forward_next.text]
            .concat()
            .matches("[redacted]")
            .count(),
        1
    );
}

#[test]
fn live_window_holds_a_new_keys_trailing_fragment_until_it_is_complete() {
    let directory = Directory::new();
    let path = directory.path().join("stream");
    let redactor = Redactor::new("fixture-new-key".into());
    std::fs::write(&path, b"head fixture-new").unwrap();
    let partial = read_window(&path, Window::Tail, false, &redactor).unwrap();
    assert_eq!(partial.text, "head ");
    assert_eq!((partial.start, partial.end, partial.length), (0, 5, 16));

    std::fs::write(&path, b"head fixture-new-key tail").unwrap();
    let next = read_window(&path, Window::After(partial.end), false, &redactor).unwrap();
    assert_eq!(next.text, "[redacted] tail");
    assert_eq!(next.start, partial.end);
    assert_eq!(next.end, next.length);
}

#[test]
fn windows_mask_single_byte_and_escaped_secrets() {
    let directory = Directory::new();
    let path = directory.path().join("stream");
    std::fs::write(&path, b"axa").unwrap();
    let one = read_window(&path, Window::Tail, true, &Redactor::new("x".into())).unwrap();
    assert_eq!(one.text, "a[redacted]a");
    assert_eq!((one.start, one.end), (0, 3));

    std::fs::write(&path, b"foo a\\nb bar").unwrap();
    let escaped = read_window(&path, Window::Tail, true, &Redactor::new("a\nb".into())).unwrap();
    assert_eq!(escaped.text, "foo [redacted] bar");
    assert_eq!((escaped.start, escaped.end), (0, 12));
}

#[test]
fn masked_windows_keep_utf8_boundaries_and_saved_byte_offsets() {
    let directory = Directory::new();
    let path = directory.path().join("stream");
    let key = "Q7vé-key";
    let mut saved = vec![b'x'; OUTPUT_LIMIT * 2];
    let key_start = OUTPUT_LIMIT - 4;
    saved[key_start..key_start + key.len()].copy_from_slice(key.as_bytes());
    saved[key_start + key.len()..].fill(b'y');
    std::fs::write(&path, saved).unwrap();
    let redactor = Redactor::new(key.into());

    let tail = read_window(&path, Window::Tail, true, &redactor).unwrap();
    assert_eq!(tail.start, OUTPUT_LIMIT as u64 + 1);
    assert_eq!(tail.end, (OUTPUT_LIMIT * 2) as u64);
    assert!(tail.text.bytes().all(|byte| byte == b'y'));
    let before = read_window(&path, Window::Before(tail.start), true, &redactor).unwrap();
    assert_eq!(before.end, tail.start);
    assert!(before.text.ends_with("[redacted]"));
    assert!(
        before.text[..before.text.len() - "[redacted]".len()]
            .bytes()
            .all(|byte| byte == b'x')
    );
}

#[test]
fn missing_streams_report_errors_without_hiding_the_process() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let observer = observer(&tools);
    let finished = bash(&tools, "printf done", &[("wait_ms", Value::number(10_000))]);
    let id = text(&finished, "process_id");
    let path = tools
        .outputs
        .resolve(&text(&finished, "stdout_file"))
        .unwrap();
    std::fs::remove_file(path).unwrap();
    let error = observer.read(&id, 0, Window::Tail).unwrap_err();
    assert!(error.contains("Could not read saved stdout"), "{error}");
    assert_eq!(listed(&observer, &id).label(), "exited 0");
    assert!(observer.read(&id, 1, Window::Tail).unwrap().text.is_empty());
    assert!(observer.read("process-1-2-3", 0, Window::Tail).is_err());
    assert!(observer.read(&id, 2, Window::Tail).is_err());
}

#[test]
fn observers_never_keep_processes_or_their_conversation_alive() {
    for drop_tools in [false, true] {
        let directory = Directory::new();
        let tools = tools(&directory, Redactor::empty());
        let observer = observer(&tools);
        let (id, _) = start(&tools, HEARTBEAT);
        wait_for(|| directory.path().join("heartbeat").exists());
        assert_eq!(observer.processes().unwrap().len(), 1);
        if drop_tools {
            drop(tools);
            assert!(heartbeat_stopped(&directory));
            assert!(observer.processes().is_none());
            continue;
        }
        tools.stop_processes();
        assert!(heartbeat_stopped(&directory));
        // The ended conversation stays ended for its observer.
        assert!(observer.processes().is_none());
        assert!(observer.read(&id, 0, Window::Tail).is_err());
        let next = tools.observe_processes(Redactor::empty());
        assert!(!next.same(&observer));
        assert!(next.same(&tools.observe_processes(Redactor::empty())));
        start(&tools, "true");
        assert!(observer.processes().is_none());
        assert_eq!(next.processes().unwrap().len(), 1);
    }
}
