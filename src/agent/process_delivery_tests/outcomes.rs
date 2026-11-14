//! An outcome on its way to the model: a requested stop, saving, a failed display and compaction.

use super::*;

#[test]
fn resume_can_recover_an_undelivered_real_outcome_without_relaunching_the_command() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![start(
                    "start",
                    &format!("printf 'run\\n' >> runs; {}", gate("go")),
                )],
            ),
        ),
        (200, completion("Started.", vec![])),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "saved-outcome",
                    "read",
                    Value::object([
                        ("path", Value::string("history:processes")),
                        ("byte_offset", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (200, completion("The saved build finished.", vec![])),
    ]);
    let mut running = agent(&directory, &fixture.endpoint);
    running.enable_sessions(home.path()).unwrap();
    running.run_turn("Build it", &mut |_| Ok(())).unwrap();
    release(&directory, "go");
    wait_for(|| running.tools.process_completion_waiting());
    let process = running.tools.process_completions(&running.redactor)[0].clone();
    running.save_session().unwrap();
    let id = running.sessions().unwrap().id();
    drop(running);

    let mut resumed = agent(&directory, &fixture.endpoint);
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert!(resumed.processes().processes().unwrap().is_empty());
    assert!(!resumed.can_continue_processes());
    resumed
        .run_turn("How did that build end?", &mut |_| Ok(()))
        .unwrap();
    let requests = fixture.finish();
    let messages = request_messages(&requests[3].body);
    let result = messages
        .iter()
        .find(|message| {
            message.get("tool_call_id").and_then(Value::as_str) == Some("saved-outcome")
        })
        .unwrap();
    let page = crate::json::parse(result.get("content").and_then(Value::as_str).unwrap()).unwrap();
    assert!(page.get("error").is_none(), "{}", page.encode());
    let saved = crate::json::parse(page.get("content").and_then(Value::as_str).unwrap()).unwrap();
    assert_eq!(saved.get("process_id"), Some(&Value::string(&process.id)));
    assert_eq!(saved.get("exit_code"), Some(&Value::number(0)));
    assert_eq!(saved.get("stdout"), Some(&Value::string("finished")));
    assert_eq!(
        saved.get("stdout_file"),
        Some(&Value::string(&process.stdout_file))
    );
    assert_eq!(saved_finishes(&resumed), 1);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("runs")).unwrap(),
        "run\n"
    );
}

#[test]
fn resume_keeps_an_undelivered_root_exit_as_history_without_relaunching() {
    let directory = Directory::new();
    let home = Directory::new();
    #[cfg(windows)]
    let launch = tool_call(
        "start",
        "exec",
        Value::object([
            (
                "executable",
                Value::string(std::env::current_exe().unwrap().to_string_lossy()),
            ),
            (
                "args",
                Value::Array(vec![
                    Value::string("--exact"),
                    Value::string("process::tests::exited_root_fixture"),
                ]),
            ),
            (
                "env",
                Value::object([
                    ("JECODE_PROCESS_FIXTURE", Value::string("root")),
                    (
                        "JECODE_PROCESS_ROOT_MARKER",
                        Value::string(directory.path().join("root-started").to_string_lossy()),
                    ),
                    (
                        "JECODE_PROCESS_CHILD_MARKER",
                        Value::string(directory.path().join("child-started").to_string_lossy()),
                    ),
                    (
                        "JECODE_PROCESS_ROOT_RELEASE",
                        Value::string(directory.path().join("go").to_string_lossy()),
                    ),
                    (
                        "JECODE_PROCESS_CHILD_RELEASE",
                        Value::string(directory.path().join("release").to_string_lossy()),
                    ),
                    (
                        "JECODE_PROCESS_MARKER",
                        Value::string(directory.path().join("survived").to_string_lossy()),
                    ),
                    ("JECODE_PROCESS_ROOT_EXIT_CODE", Value::string("7")),
                ]),
            ),
            ("wait_ms", Value::number(0)),
        ]),
    );
    #[cfg(unix)]
    let launch = start(
        "start",
        &format!("{}; (while :; do sleep 0.05; done) & exit 7", gate("go")),
    );
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![launch])),
        (200, completion("Started.", vec![])),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "saved-root",
                    "read",
                    Value::object([
                        ("path", Value::string("history:processes")),
                        ("byte_offset", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (
            200,
            completion("The root exited while a child kept running.", vec![]),
        ),
    ]);
    let mut running = agent(&directory, &fixture.endpoint);
    running.enable_sessions(home.path()).unwrap();
    running
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    running
        .run_turn("Start the command", &mut |_| Ok(()))
        .unwrap();
    #[cfg(windows)]
    wait_for(|| directory.path().join("child-started").exists());
    release(&directory, "go");
    wait_for(|| running.tools.process_root_notice_waiting());
    let process_id = running.tools.process_root_notices(&running.redactor)[0]
        .id
        .clone();
    running.save_session().unwrap();
    let id = running.sessions().unwrap().id();
    drop(running);

    let mut resumed = agent(&directory, &fixture.endpoint);
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert!(resumed.processes().processes().unwrap().is_empty());
    assert!(!resumed.can_continue_processes());
    assert!(
        resumed
            .sessions()
            .unwrap()
            .snapshot()
            .records()
            .iter()
            .any(|record| { matches!(record, crate::sessions::Record::ProcessRootExited { .. }) })
    );
    resumed
        .run_turn("What did that process do?", &mut |_| Ok(()))
        .unwrap();
    let requests = fixture.finish();
    let restored_messages = request_messages(&requests[3].body);
    let message = restored_messages
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some("saved-root"))
        .unwrap();
    let page = crate::json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap();
    let saved = crate::json::parse(page.get("content").and_then(Value::as_str).unwrap()).unwrap();
    assert_eq!(
        saved.get("type").and_then(Value::as_str),
        Some("process_root_exited")
    );
    assert_eq!(saved.get("process_id"), Some(&Value::string(process_id)));
}

#[test]
fn a_stop_the_user_asked_for_reaches_the_next_request_without_starting_one() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("start", "sleep 20")])),
        (200, completion("Started.", vec![])),
        (200, completion("Understood.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.run_turn("Run it", &mut |_| Ok(())).unwrap();
    let observer = agent.processes();
    let id = observer.processes().unwrap()[0].id.clone();
    observer.request_stop(&id).unwrap();
    wait_for(|| !observer.completions_since(0).1.is_empty());
    assert!(!agent.can_continue_processes());
    agent.run_turn("What happened?", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(deliveries(&requests[2].body), 1);
    assert!(crate::test_support::latest_state(&requests[2].body).contains(": stopped after "));
}

#[test]
fn saving_keeps_an_outcome_no_request_has_carried_and_delivery_adds_none() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("start", &gate("go"))])),
        (200, completion("Started.", vec![])),
        (200, completion("Seen.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.enable_sessions(home.path()).unwrap();
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    release(&directory, "go");
    wait_for(|| agent.tools.process_completion_waiting());
    agent.save_session().unwrap();
    let records = |agent: &Agent| {
        agent
            .sessions()
            .unwrap()
            .snapshot()
            .records()
            .iter()
            .filter(|record| matches!(record, crate::sessions::Record::ProcessFinished { .. }))
            .count()
    };
    assert_eq!(records(&agent), 1);
    // Saved is not delivered: the model still hears about it, and nothing doubles.
    assert!(agent.can_continue_processes());
    agent.run_turn("And now?", &mut |_| Ok(())).unwrap();
    assert_eq!(records(&agent), 1);
    let requests = fixture.finish();
    assert_eq!(deliveries(&requests[2].body), 1);
}

#[test]
fn a_failed_display_keeps_the_outcome_owed_and_saves_every_completion() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![start("one", &gate("go")), start("two", &gate("go"))],
            ),
        ),
        (200, completion("Started.", vec![])),
        (200, completion("Lost with the display.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    release(&directory, "go");
    wait_for(|| agent.processes().completions_since(0).1.len() == 2);
    let result = agent.continue_processes(&mut |event| match event {
        Event::ProcessFinished { .. } => Err("The display closed".into()),
        _ => Ok(()),
    });
    assert!(result.is_err());
    assert_eq!(saved_finishes(&agent), 2);
    assert!(agent.tools.process_completion_waiting());
    fixture.finish();
}

#[test]
fn a_summary_of_portions_keeps_an_outcome_only_state_carried() {
    let directory = Directory::new();
    let rejected = Value::object([(
        "error",
        Value::object([
            ("code", Value::string("context_length_exceeded")),
            ("message", Value::string("Maximum context length exceeded")),
        ]),
    )]);
    // A rejection compacts by portions, however many this prompt takes.
    let mut responses = vec![
        (200, completion("", vec![start("start", &gate("go"))])),
        (200, completion("Started.", vec![])),
        (200, completion("It passed.", vec![])),
        (400, rejected),
    ];
    responses.extend((0..8).map(|_| (200, completion("Kept.", vec![]))));
    let fixture = HttpFixture::new(responses);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.set_auto_compact(90);
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    release(&directory, "go");
    wait_for(|| agent.can_continue_processes());
    agent.continue_processes(&mut |_| Ok(())).unwrap();
    let delivered = agent.context.states.last().unwrap().before;
    // Larger than the recent window, so the boundary falls after the state.
    let fixed = agent.current_system_message().encode().len()
        + crate::tools::encoded_size()
        + agent.request_environment().len();
    let prompt = "next objective ".repeat(fixed * 4 / 15);
    agent.run_turn(&prompt, &mut |_| Ok(())).unwrap();
    assert!(agent.context.from > delivered);
    let requests = fixture.finish_received();
    assert!(
        requests
            .iter()
            .filter(|request| request.body.get("tools").is_none())
            .any(|request| request.body.encode().contains(SECTION))
    );
}
