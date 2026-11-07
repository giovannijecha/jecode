//! Launch waits, yielding and moving a wait into the background.

use super::*;

pub(super) fn observer(tools: &Tools) -> Observer {
    tools.observe_processes(Redactor::empty())
}

pub(super) fn only(observer: &Observer) -> Process {
    let processes = observer.processes().unwrap();
    assert_eq!(processes.len(), 1, "{processes:?}");
    processes.into_iter().next().unwrap()
}

pub(super) fn fast_yield(tools: &Tools) {
    tools.set_default_wait(Duration::from_millis(50));
}

/// Runs `bash` on its own thread, for a call that blocks while the test acts.
pub(super) fn launch(
    tools: &Arc<Tools>,
    command: &str,
    cancellation: &Cancellation,
) -> JoinHandle<Value> {
    let (tools, command, cancellation) =
        (Arc::clone(tools), command.to_owned(), cancellation.clone());
    thread::spawn(move || {
        let arguments = Value::object([("command", Value::string(command))]);
        tools.execute_with_cancel("bash", &arguments.encode(), &cancellation)
    })
}

#[test]
fn a_command_is_discoverable_and_controllable_during_its_initial_wait() {
    let directory = Directory::new();
    let tools = Arc::new(tools(&directory, Redactor::empty()));
    tools.set_default_wait(Duration::from_secs(30));
    let observer = observer(&tools);
    let cancellation = Cancellation::default();
    let call = launch(&tools, "printf started; sleep 20", &cancellation);
    wait_for(|| observer.processes().is_some_and(|list| !list.is_empty()));
    let running = only(&observer);
    assert_eq!(running.status, "running");
    assert!(running.awaited);
    assert_eq!(running.command, "printf started; sleep 20");
    // The footer counts only processes whose launching call has returned.
    assert_eq!(observer.background_count(), 0);
    wait_for(|| {
        observer
            .read(&running.id, 0, Window::Tail)
            .is_ok_and(|page| page.text == "started")
    });
    let listed = tools.execute("process", r#"{"action":"list"}"#);
    assert_eq!(
        listed.get("processes").unwrap().as_array().unwrap().len(),
        1
    );

    // An individual stop ends it, and the waiting call reports the final state.
    observer.request_stop(&running.id).unwrap();
    let result = call.join().unwrap();
    assert_eq!(
        result.get("cancelled"),
        Some(&Value::Bool(true)),
        "{result:?}"
    );
    assert_eq!(text(&result, "stdout"), "started");
    assert!(observer.completions_since(0).1.is_empty());
}

#[test]
fn the_documented_default_wait_is_the_one_in_use() {
    assert_eq!(DEFAULT_WAIT, Duration::from_secs(10));
    for name in ["bash", "exec"] {
        let definition = crate::tools::definitions()
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
            .unwrap();
        let description = definition.get("description").and_then(Value::as_str);
        assert!(description.unwrap().contains("10 seconds"), "{name}");
    }
}

#[test]
fn release_between_registration_and_wait_is_not_lost_or_replayed() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let (id, _) = start(&tools, &gate("go"));
    let observer = observer(&tools);
    let entry = tools
        .processes
        .scope()
        .entries
        .lock()
        .unwrap()
        .iter()
        .find(|entry| entry.id == id)
        .unwrap()
        .clone();

    // Force the scheduler window between a call registering its wait and
    // entering the wait loop. The release must still return that call.
    let first = entry.begin_wait();
    observer.move_to_background(&id).unwrap();
    assert_eq!(
        entry.wait_held(
            Duration::from_secs(1),
            &Cancellation::default(),
            Some(first)
        ),
        WaitEnd::Released
    );
    entry
        .report(WaitEnd::Released, DEFAULT_OUTPUT_LIMIT, OutputMode::Next)
        .unwrap();

    // The next call starts after the release and must wait on its own clock.
    let second = entry.begin_wait();
    assert_ne!(first, second);
    assert_eq!(
        entry.wait_held(
            Duration::from_millis(30),
            &Cancellation::default(),
            Some(second)
        ),
        WaitEnd::Elapsed
    );
    entry
        .report(WaitEnd::Elapsed, DEFAULT_OUTPUT_LIMIT, OutputMode::Next)
        .unwrap();
    release(&directory, "go");
    wait_for(|| tools.process_completion_waiting());
}

#[test]
fn a_yielding_command_keeps_its_prelaunch_checkpoint_receipt() {
    use crate::checkpoints::session::Session;

    let home = Directory::new();
    let root = Directory::new();
    let session = Session::open(home.path(), root.path());
    session.begin("model");
    let mut tools = tools(&root, Redactor::empty());
    tools.configure_checkpoints(std::sync::Arc::clone(&session));
    fast_yield(&tools);
    let started = bash(
        &tools,
        &gate("go"),
        &[("protect", Value::Array(vec![Value::string("generated/")]))],
    );
    assert_eq!(
        started.get("yielded"),
        Some(&Value::Bool(true)),
        "{started:?}"
    );
    let checkpoint = started.get("checkpoint").unwrap();
    assert_eq!(
        checkpoint.get("status").and_then(Value::as_str),
        Some("saved")
    );
    release(&root, "go");
    wait_for(|| tools.process_completion_waiting());
}

#[test]
fn a_short_command_returns_its_final_result_with_no_process_id() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let observer = observer(&tools);
    let result = bash(&tools, "printf out; printf err >&2; exit 3", &[]);
    assert!(result.get("process_id").is_none(), "{result:?}");
    assert!(result.get("yielded").is_none());
    assert_eq!(number(&result, "exit_code"), 3);
    assert_eq!(text(&result, "stdout"), "out");
    assert_eq!(text(&result, "stderr"), "err");
    assert_eq!(number(&result, "stdout_bytes"), 3);
    // The receipt carried the outcome, so nothing is announced after it.
    assert!(observer.completions_since(0).1.is_empty());
    assert!(!tools.process_completion_waiting());
    assert_eq!(only(&observer).label(), "exited 3");
}

#[test]
fn a_long_command_yields_keeps_running_and_never_restarts() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    fast_yield(&tools);
    let observer = observer(&tools);
    let started = Instant::now();
    let first = bash(
        &tools,
        &format!(
            "printf 'run\\n' >> runs; printf early; {}; printf late",
            gate("go")
        ),
        &[],
    );
    assert!(started.elapsed() < Duration::from_secs(8), "{first:?}");
    assert_eq!(text(&first, "status"), "running");
    assert_eq!(first.get("yielded"), Some(&Value::Bool(true)));
    let id = text(&first, "process_id");
    assert_eq!(observer.background_count(), 1);
    assert!(!only(&observer).awaited);

    // Other work continues while it runs, and ends within its own wait.
    tools.set_default_wait(Duration::from_secs(30));
    let read = bash(&tools, "printf independent", &[]);
    assert_eq!(text(&read, "stdout"), "independent");
    assert_eq!(
        observer.processes().unwrap()[0].status,
        "running",
        "the yielded command keeps running"
    );

    release(&directory, "go");
    wait_for(|| tools.process_completion_waiting());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("runs")).unwrap(),
        "run\n"
    );
    let completions = tools.process_completions(&Redactor::empty());
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].id, id);
    assert_eq!(completions[0].status, "exited");
    assert_eq!(completions[0].exit_code, Some(0));
    // The receipt and the poll hold all the output between them, once.
    let rest = process(&tools, "poll", &id, None);
    assert_eq!(text(&first, "stdout") + &text(&rest, "stdout"), "earlylate");
}

#[test]
fn an_explicit_wait_elapses_without_ending_the_command_and_a_timeout_ends_it() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let waited = bash(&tools, &gate("go"), &[("wait_ms", Value::number(30))]);
    assert_eq!(text(&waited, "status"), "running");
    assert!(waited.get("yielded").is_none(), "{waited:?}");
    release(&directory, "go");
    let id = text(&waited, "process_id");
    assert_eq!(
        text(&process(&tools, "poll", &id, Some(10_000)), "status"),
        "exited"
    );

    // The deadline and the wait are separate: it ends during a long wait.
    let bounded = bash(
        &tools,
        "sleep 20",
        &[
            ("timeout_seconds", Value::number(1)),
            ("wait_ms", Value::number(10_000)),
        ],
    );
    assert_eq!(text(&bounded, "status"), "timed_out");
}

#[test]
fn moving_a_wait_to_the_background_returns_control_without_relaunching() {
    let directory = Directory::new();
    let tools = Arc::new(tools(&directory, Redactor::empty()));
    tools.set_default_wait(Duration::from_secs(60));
    let observer = observer(&tools);
    let cancellation = Cancellation::default();
    let call = launch(
        &tools,
        &format!("printf 'run\\n' >> runs; {}", gate("go")),
        &cancellation,
    );
    wait_for(|| {
        observer
            .processes()
            .is_some_and(|list| list.iter().any(|process| process.awaited))
    });
    let id = only(&observer).id;
    wait_for(|| directory.path().join("runs").exists());
    observer.move_to_background(&id).unwrap();
    let moved = call.join().unwrap();
    assert_eq!(moved.get("moved_to_background"), Some(&Value::Bool(true)));
    assert_eq!(moved.get("yielded"), Some(&Value::Bool(true)));
    assert_eq!(text(&moved, "status"), "running");
    assert_eq!(text(&moved, "process_id"), id);
    assert_eq!(observer.background_count(), 1);
    // Cancelling the turn afterwards does not reach a process it released.
    cancellation.cancel();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(only(&observer).status, "running");
    assert!(observer.move_to_background(&id).is_err());

    release(&directory, "go");
    wait_for(|| tools.process_completion_waiting());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("runs")).unwrap(),
        "run\n"
    );
}

#[test]
fn a_poll_wait_can_also_be_released_leaving_the_process_running() {
    let directory = Directory::new();
    let tools = Arc::new(tools(&directory, Redactor::empty()));
    let (id, _) = start(&tools, &gate("go"));
    let observer = observer(&tools);
    let poller = {
        let (tools, id) = (Arc::clone(&tools), id.clone());
        thread::spawn(move || process(&tools, "poll", &id, Some(60_000)))
    };
    wait_for(|| only(&observer).awaited);
    observer.move_to_background(&id).unwrap();
    let polled = poller.join().unwrap();
    assert_eq!(polled.get("moved_to_background"), Some(&Value::Bool(true)));
    assert_eq!(text(&polled, "status"), "running");
    release(&directory, "go");
    wait_for(|| tools.process_completion_waiting());
}

#[test]
fn interrupting_a_foreground_wait_stops_it_and_an_explicit_wait_preserves_it() {
    let directory = Directory::new();
    let tools = Arc::new(tools(&directory, Redactor::empty()));
    tools.set_default_wait(Duration::from_secs(60));
    let observer = observer(&tools);
    let cancellation = Cancellation::default();
    let call = launch(&tools, "printf partial; sleep 20", &cancellation);
    wait_for(|| {
        observer.processes().is_some_and(|list| {
            list.first().is_some_and(|first| {
                observer
                    .read(&first.id, 0, Window::Tail)
                    .is_ok_and(|page| page.text == "partial")
            })
        })
    });
    let started = Instant::now();
    cancellation.cancel();
    let result = call.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(8));
    assert_eq!(
        result.get("cancelled"),
        Some(&Value::Bool(true)),
        "{result:?}"
    );
    assert_eq!(result.get("timed_out"), Some(&Value::Bool(false)));
    assert_eq!(text(&result, "stdout"), "partial");
    assert!(observer.completions_since(0).1.is_empty());

    // The same interruption of an explicit wait leaves the process alone.
    let cancellation = Cancellation::default();
    let explicit = {
        let (tools, cancellation) = (Arc::clone(&tools), cancellation.clone());
        let command = format!("{}; printf finished", gate("go"));
        thread::spawn(move || {
            let arguments = Value::object([
                ("command", Value::string(command)),
                ("wait_ms", Value::number(60_000)),
            ]);
            tools.execute_with_cancel("bash", &arguments.encode(), &cancellation)
        })
    };
    wait_for(|| {
        observer
            .processes()
            .is_some_and(|list| list.iter().any(|process| process.awaited))
    });
    cancellation.cancel();
    let waited = explicit.join().unwrap();
    assert_eq!(waited.get("wait_cancelled"), Some(&Value::Bool(true)));
    assert_eq!(text(&waited, "status"), "running");
    release(&directory, "go");
    let id = text(&waited, "process_id");
    let finished = process(&tools, "poll", &id, Some(10_000));
    assert_eq!(text(&finished, "stdout"), "finished");
}
