use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::{
    fs,
    sync::{Barrier, Condvar, Mutex},
    time::Duration,
};

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "fixture".into(),
        arguments: "{}".into(),
    }
}

#[test]
fn fast_completion_reaches_controller_before_the_first_worker_finishes() {
    let started = Barrier::new(2);
    let release = (Mutex::new(false), Condvar::new());
    let mut order = Vec::new();
    run_workers(
        &[call("slow"), call("fast")],
        vec![Begun::default(), Begun::default()],
        Clock::new(),
        &|call, _| {
            started.wait();
            if call.id == "slow" {
                let (released, timeout) = release
                    .1
                    .wait_timeout_while(
                        release.0.lock().unwrap(),
                        Duration::from_secs(5),
                        |released| !*released,
                    )
                    .unwrap();
                assert!(!timeout.timed_out() && *released);
            }
            Execution {
                result: Value::object([("id", Value::string(&call.id))]),
                change: None,
            }
        },
        |index, result, _| {
            assert!(result.result.get("error").is_none());
            order.push(index);
            if index == 1 {
                *release.0.lock().unwrap() = true;
                release.1.notify_all();
            }
        },
    );
    assert_eq!(order, [1, 0]);
}

#[test]
fn worker_panic_after_an_effect_is_an_unknown_outcome_without_reexecution() {
    let directory = Directory::new();
    let path = directory.path().join("effect");
    let mut results = Vec::new();
    run_workers(
        &[call("one")],
        vec![Begun::default()],
        Clock::new(),
        &|_, _| {
            fs::write(&path, "once").unwrap();
            panic!("fixture failure after effect");
        },
        |_, result, _| results.push(result.result),
    );
    assert_eq!(fs::read_to_string(path).unwrap(), "once");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("outcome"), Some(&Value::string("unknown")));
}

pub(super) fn command(id: &str, command: &str, path: &str) -> Value {
    tool_call(
        id,
        "bash",
        Value::object([
            ("command", Value::string(command)),
            ("parallel", Value::Bool(true)),
            ("timeout_seconds", Value::number(5)),
            ("protect", Value::Array(vec![Value::string(path)])),
        ]),
    )
}

#[test]
fn independent_commands_overlap_and_a_dependent_read_waits_for_both() {
    let project = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("a.txt"), "old-a").unwrap();
    fs::write(project.path().join("b.txt"), "old-b").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    command(
                        "first",
                        "printf ready > ready-a; for ((n=0;n<200;n++)); do if test -e ready-b; then printf alpha > a.txt; exit 0; fi; sleep 0.01; done; exit 71",
                        "a.txt",
                    ),
                    command(
                        "second",
                        "printf ready > ready-b; for ((n=0;n<200;n++)); do if test -e ready-a; then printf beta > b.txt; exit 0; fi; sleep 0.01; done; exit 72",
                        "b.txt",
                    ),
                    tool_call(
                        "dependent",
                        "read",
                        Value::object([
                            ("path", Value::string("a.txt")),
                            ("parallel", Value::Bool(false)),
                        ]),
                    ),
                ],
            ),
        ),
        (200, completion("Done", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    agent.use_checkpoints(crate::checkpoints::session::Session::offline(
        home.path(),
        project.path(),
    ));
    agent
        .run_turn(
            "Run independent jobs, then inspect their results",
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(project.path().join("a.txt")).unwrap(),
        "alpha"
    );
    assert_eq!(
        fs::read_to_string(project.path().join("b.txt")).unwrap(),
        "beta"
    );
    let messages = agent.messages.lock().unwrap();
    let receipts = messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .collect::<Vec<_>>();
    assert_eq!(
        receipts
            .iter()
            .map(|message| message.get("tool_call_id").and_then(Value::as_str).unwrap())
            .collect::<Vec<_>>(),
        ["first", "second", "dependent"]
    );
    for receipt in &receipts[..2] {
        let value =
            crate::json::parse(receipt.get("content").and_then(Value::as_str).unwrap()).unwrap();
        assert_eq!(value.get("exit_code"), Some(&Value::number(0)));
        assert!(value.get("checkpoint").is_some());
    }
    assert!(
        receipts[2]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("alpha")
    );
    drop(messages);
    let saved = agent.sessions().unwrap().snapshot();
    assert!(
        !saved.pending.active
            && saved.pending.batch.is_empty()
            && saved.pending.completed.is_empty()
    );
    assert_eq!(fixture.finish().len(), 2);
}
