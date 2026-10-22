use super::*;

#[test]
fn bash_shows_its_point_first_and_reports_what_it_changed() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let arguments = Value::object([
        (
            "command",
            Value::string("printf made > made.txt; rm notes.txt; exit 3"),
        ),
        (
            "protect",
            Value::Array(vec![Value::string("made.txt"), Value::string("notes.txt")]),
        ),
    ])
    .encode();
    let begun = tools.begin("bash", &arguments);
    let note = begun.note().unwrap();
    assert!(note.starts_with("checkpoint cp-"), "{note}");
    let result = tools
        .execute_begun(begun, "bash", &arguments, &Cancellation::default())
        .result;
    assert_eq!(result.get("exit_code").and_then(Value::as_usize), Some(3));
    let checkpoint = result.get("checkpoint").unwrap();
    assert_eq!(text(checkpoint, &["status"]), "saved");
    assert_eq!(
        note,
        format!("checkpoint {}", text(checkpoint, &["before"]))
    );
    assert_eq!(
        checkpoint.get("changed"),
        Some(&Value::Array(vec![
            Value::string("made.txt"),
            Value::string("notes.txt")
        ]))
    );

    // A command that times out still reports the changes it made.
    let timed = call(
        tools,
        "bash",
        &[
            (
                "command",
                Value::string("printf partial > partial.txt; sleep 10"),
            ),
            ("protect", Value::Array(vec![Value::string("partial.txt")])),
            ("timeout_seconds", Value::number(1)),
        ],
    );
    assert_eq!(
        timed.get("timed_out"),
        Some(&Value::Bool(true)),
        "{timed:?}"
    );
    assert!(
        timed
            .get("checkpoint")
            .unwrap()
            .encode()
            .contains("partial.txt"),
        "{timed:?}"
    );

    // A command without a declared scope reports no saved preimage.
    let first = call(tools, "bash", &[("command", Value::string("ls"))]);
    let second = call(tools, "bash", &[("command", Value::string("ls"))]);
    assert_eq!(text(&first, &["checkpoint", "status"]), "none");
    assert_eq!(text(&second, &["checkpoint", "status"]), "none");
    assert!(first.get("checkpoint").unwrap().get("before").is_none());
}

#[test]
fn successful_bash_keeps_its_before_state_until_the_model_accepts_the_group() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let ran = call(
        tools,
        "bash",
        &[
            (
                "command",
                Value::string("printf second > notes.txt; printf new > created.txt"),
            ),
            (
                "protect",
                Value::Array(vec![
                    Value::string("notes.txt"),
                    Value::string("created.txt"),
                ]),
            ),
        ],
    );
    assert_eq!(ran.get("exit_code").and_then(Value::as_usize), Some(0));
    let checkpoint = ran.get("checkpoint").unwrap();
    assert_eq!(text(checkpoint, &["status"]), "saved", "{ran:?}");
    let before = text(checkpoint, &["before"]).to_string();
    let group = text(checkpoint, &["group"]).to_string();
    let after = text(checkpoint, &["after"]).to_string();
    let status = call(tools, "checkpoint", &[("action", Value::string("status"))]);
    assert_eq!(text(&status, &["active_group"]), group);

    let restored = call(
        tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(&before)),
            (
                "paths",
                Value::Array(vec![
                    Value::string("notes.txt"),
                    Value::string("created.txt"),
                ]),
            ),
            ("expected", Value::string(&after)),
        ],
    );
    assert!(restored.get("error").is_none(), "{restored:?}");
    assert_eq!(
        restored
            .get("restored")
            .and_then(Value::as_array)
            .map(<[Value]>::len),
        Some(2),
        "{restored:?}"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.path().join("notes.txt")).unwrap(),
        "first\n"
    );
    assert!(!fixture.root.path().join("created.txt").exists());

    let accepted = call(tools, "checkpoint", &[("action", Value::string("accept"))]);
    assert_eq!(text(&accepted, &["status"]), "accepted", "{accepted:?}");
    assert_eq!(text(&accepted, &["group"]), group);
    let intermediate = call(
        tools,
        "checkpoint",
        &[
            ("action", Value::string("inspect")),
            ("ref", Value::string(&before)),
            ("paths", Value::Array(vec![Value::string("notes.txt")])),
        ],
    );
    assert!(intermediate.get("error").is_some(), "{intermediate:?}");
    let inspected = call(
        tools,
        "checkpoint",
        &[
            ("action", Value::string("inspect")),
            ("ref", Value::string(&group)),
            ("paths", Value::Array(vec![Value::string("notes.txt")])),
        ],
    );
    assert!(inspected.get("error").is_none(), "{inspected:?}");
}

#[test]
fn explicit_capture_records_precise_preimages_before_bash() {
    let fixture = fixture();
    let workspace = fixture.tools.checkpoints().unwrap().workspace().unwrap();
    workspace
        .configure(Some(1 << 20), Some(512 << 10), "test")
        .unwrap();
    fs::create_dir_all(fixture.root.path().join("data")).unwrap();
    for number in 0..4 {
        fs::write(
            fixture.root.path().join(format!("data/{number}.bin")),
            vec![number as u8; 384 << 10],
        )
        .unwrap();
    }
    fs::write(fixture.root.path().join("work.txt"), b"original\0bytes").unwrap();
    let captured = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("capture")),
            (
                "paths",
                Value::Array(vec![
                    Value::string("work.txt"),
                    Value::string("created.txt"),
                ]),
            ),
        ],
    );
    assert_eq!(
        captured.get("saved").and_then(Value::as_usize),
        Some(2),
        "{captured:?}"
    );
    let before = text(&captured, &["ref"]).to_string();
    let ran = call(
        &fixture.tools,
        "bash",
        &[
            (
                "command",
                Value::string("printf changed > work.txt; printf new > created.txt"),
            ),
            (
                "protect",
                Value::Array(vec![
                    Value::string("work.txt"),
                    Value::string("created.txt"),
                ]),
            ),
        ],
    );
    assert_eq!(
        ran.get("exit_code").and_then(Value::as_usize),
        Some(0),
        "{ran:?}"
    );
    assert_eq!(text(&ran, &["checkpoint", "status"]), "saved");
    assert!(ran.get("checkpoint").unwrap().get("warning").is_none());
    let conflicted = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(&before)),
            (
                "paths",
                Value::Array(vec![
                    Value::string("work.txt"),
                    Value::string("created.txt"),
                ]),
            ),
        ],
    );
    assert!(
        conflicted
            .get("conflicts")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty()),
        "{conflicted:?}"
    );
    let restored = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(&before)),
            (
                "paths",
                Value::Array(vec![
                    Value::string("work.txt"),
                    Value::string("created.txt"),
                ]),
            ),
            ("expected", Value::string("latest")),
        ],
    );
    assert_eq!(
        restored
            .get("restored")
            .and_then(Value::as_array)
            .map(<[Value]>::len),
        Some(2),
        "{restored:?}"
    );
    assert_eq!(
        fs::read(fixture.root.path().join("work.txt")).unwrap(),
        b"original\0bytes"
    );
    assert!(!fixture.root.path().join("created.txt").exists());
}

#[test]
fn background_processes_are_reported_while_running_and_captured_later() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let started = call(
        tools,
        "bash",
        &[
            (
                "command",
                Value::string("until [ -e go ]; do sleep 0.02; done; printf late > late.txt"),
            ),
            ("wait_ms", Value::number(0)),
            ("protect", Value::Array(vec![Value::string("late.txt")])),
        ],
    );
    assert_eq!(text(&started, &["status"]), "running");
    assert!(text(&started, &["checkpoint", "note"]).contains("still running"));
    let status = call(tools, "checkpoint", &[("action", Value::string("status"))]);
    assert_eq!(
        status.get("background_processes").and_then(Value::as_usize),
        Some(1),
        "{status:?}"
    );

    fs::write(fixture.root.path().join("go"), "").unwrap();
    let id = text(&started, &["process_id"]).to_string();
    wait_for(|| {
        let polled = call(
            tools,
            "process",
            &[
                ("action", Value::string("poll")),
                ("process_id", Value::string(&id)),
            ],
        );
        text(&polled, &["status"]) == "exited"
    });
    wait_for(|| {
        let status = call(tools, "checkpoint", &[("action", Value::string("status"))]);
        status.get("background_processes").and_then(Value::as_usize) == Some(0)
    });
    // The process monitor observes the protected path when the process exits.
    let listed = call(
        tools,
        "checkpoint",
        &[
            ("action", Value::string("list")),
            ("paths", Value::Array(vec![Value::string("late.txt")])),
        ],
    );
    assert!(listed.encode().contains("late.txt"), "{listed:?}");
    assert!(
        listed.get("paths").unwrap().as_array().unwrap()[0]
            .get("total")
            .and_then(Value::as_usize)
            .is_some_and(|count| count > 0),
        "{listed:?}"
    );
    let version = call(tools, "read", &[("path", Value::string("late.txt"))]);
    assert!(version.encode().contains("late"), "{version:?}");
}
