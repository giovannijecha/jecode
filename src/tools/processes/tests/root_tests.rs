//! A root that exits while its descendants keep the supervised boundary open.
use super::*;

// The root exits with 7 at once; the background subshell is a descendant.
const LONG_CHILD: &str =
    "(while :; do printf x >> heartbeat; sleep 0.05; done) & printf 'ready\\n'; exit 7";

fn root(value: &Value) -> &Value {
    value
        .get("root_process")
        .unwrap_or_else(|| panic!("root_process missing from {value:?}"))
}

fn assert_root_exit(value: &Value) {
    let root = root(value);
    assert_eq!(root.get("exit_code"), Some(&Value::number(7)), "{value:?}");
    assert_eq!(text(root, "status"), "exited");
    assert!(
        root.get("pid")
            .and_then(Value::as_usize)
            .is_some_and(|pid| pid > 0)
    );
}

#[test]
fn a_launch_reports_its_root_exit_while_descendants_keep_running() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let started = Instant::now();
    // Foreground: no wait_ms, so the call would otherwise hold for the default wait.
    let first = bash(
        &tools,
        LONG_CHILD,
        &[("timeout_seconds", Value::number(60))],
    );
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "the root's exit must end the launch wait: {:?}",
        started.elapsed()
    );
    assert_eq!(text(&first, "status"), "running", "{first:?}");
    assert_eq!(first.get("yielded"), Some(&Value::Bool(true)));
    assert!(
        first.get("exit_code").is_none(),
        "the tree has not ended: {first:?}"
    );
    assert_root_exit(&first);
    assert!(text(&first, "stdout").contains("ready"));
    let waiting_for = text(&first, "waiting_for");
    #[cfg(windows)]
    {
        assert_eq!(waiting_for, "supervised descendant processes");
        let descendants = first.get("descendants").unwrap();
        assert!(number(descendants, "running") >= 1, "{first:?}");
        let processes = descendants
            .get("processes")
            .and_then(Value::as_array)
            .unwrap();
        assert!(
            processes
                .iter()
                .any(|process| !text(process, "image").is_empty()),
            "{first:?}"
        );
    }
    #[cfg(unix)]
    assert!(waiting_for.contains("output"), "{first:?}");
    let id = text(&first, "process_id");

    // The facts stay visible to later observation while the tree runs.
    let poll = process(&tools, "poll", &id, Some(0));
    assert_eq!(text(&poll, "status"), "running");
    assert_root_exit(&poll);
    let listed = tools.execute(
        "process",
        &Value::object([("action", Value::string("list"))]).encode(),
    );
    let entry = &listed.get("processes").and_then(Value::as_array).unwrap()[0];
    assert_root_exit(entry);

    // Ownership is unchanged: stop still ends the descendants.
    let stopped = process(&tools, "stop", &id, None);
    assert_eq!(text(&stopped, "status"), "stopped", "{stopped:?}");
    assert_root_exit(&stopped);
    assert!(stopped.get("waiting_for").is_none());
    assert!(heartbeat_stopped(&directory), "descendant survived stop");
}

#[test]
fn a_short_lived_descendant_still_gives_an_ordinary_final_result() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let result = bash(
        &tools,
        "(sleep 0.1; printf 'child done\\n') & printf 'root done\\n'; exit 7",
        &[("timeout_seconds", Value::number(60))],
    );
    assert!(result.get("process_id").is_none(), "{result:?}");
    assert_eq!(
        result.get("exit_code"),
        Some(&Value::number(7)),
        "{result:?}"
    );
    assert!(result.get("root_process").is_none(), "{result:?}");
    assert!(text(&result, "stdout").contains("child done"), "{result:?}");
}

#[test]
fn a_poll_ends_when_the_root_exits_during_it() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let (id, first) = start(
        &tools,
        &format!(
            "{}; (while :; do printf x >> heartbeat; sleep 0.05; done) & exit 7",
            gate("go")
        ),
    );
    assert!(first.get("root_process").is_none(), "{first:?}");
    release(&directory, "go");
    let started = Instant::now();
    let poll = process(&tools, "poll", &id, Some(30_000));
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(text(&poll, "status"), "running", "{poll:?}");
    assert_root_exit(&poll);
    // A later wait knows the root already ended and waits for the tree.
    let again = process(&tools, "poll", &id, Some(600));
    assert_eq!(text(&again, "status"), "running");
    process(&tools, "stop", &id, None);
    assert!(heartbeat_stopped(&directory));
}

#[test]
fn a_root_that_exits_after_the_launch_wait_gets_one_notice() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let (id, first) = start(
        &tools,
        &format!(
            "{}; (while :; do printf x >> heartbeat; sleep 0.05; done) & exit 7",
            gate("go")
        ),
    );
    assert!(first.get("root_process").is_none());
    let observer = tools.observe_processes(Redactor::empty());
    release(&directory, "go");
    wait_for(|| tools.process_root_notice_waiting());
    let notices = tools.process_root_notices(&Redactor::empty());
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].id, id);
    assert_root_exit(&notices[0].value());
    assert!(notices[0].report().contains("root exited with code 7"));
    assert_eq!(observer.root_notices_since(0).1.len(), 1);
    tools.process_roots_reported(std::slice::from_ref(&id));
    assert!(!tools.process_root_notice_waiting());
    process(&tools, "stop", &id, None);
    assert!(heartbeat_stopped(&directory));
    assert!(tools.process_root_notices(&Redactor::empty()).is_empty());
}

#[test]
fn a_timeout_after_the_root_exit_names_both_outcomes() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let (id, _) = {
        let result = bash(
            &tools,
            LONG_CHILD,
            &[
                ("timeout_seconds", Value::number(2)),
                ("wait_ms", Value::number(0)),
            ],
        );
        (text(&result, "process_id"), result)
    };
    wait_for(|| tools.process_completion_waiting());
    let completions = tools.process_completions(&Redactor::empty());
    let completion = completions
        .iter()
        .find(|completion| completion.id == id)
        .unwrap();
    assert_eq!(completion.status, "timed_out");
    let report = completion.report();
    assert!(
        report.contains("timed out") && report.contains("had exited with code 7"),
        "{report}"
    );
    let poll = process(&tools, "poll", &id, Some(0));
    assert_eq!(text(&poll, "status"), "timed_out");
    assert_root_exit(&poll);
    assert!(
        heartbeat_stopped(&directory),
        "descendant survived the timeout"
    );
}
