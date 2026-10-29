use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::fs;

fn fixture_agent(project: &Directory, home: &Directory, fixture: &HttpFixture) -> Agent {
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.use_checkpoints(crate::checkpoints::session::Session::offline(
        home.path(),
        project.path(),
    ));
    agent
}

fn write(id: &str, path: &str) -> Value {
    tool_call(
        id,
        "write",
        Value::object([
            ("path", Value::string(path)),
            ("content", Value::string(id)),
            ("parallel", Value::Bool(true)),
        ]),
    )
}

fn results(agent: &Agent) -> Vec<(String, Value)> {
    agent
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .map(|message| {
            (
                message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap()
                    .into(),
                crate::json::parse(message.get("content").and_then(Value::as_str).unwrap())
                    .unwrap(),
            )
        })
        .collect()
}

#[test]
fn one_operation_failure_does_not_undo_or_cancel_an_independent_success() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    write("good", "good.txt"),
                    tool_call(
                        "bad",
                        "edit",
                        Value::object([
                            ("path", Value::string("absent.txt")),
                            ("old_text", Value::string("old")),
                            ("new_text", Value::string("new")),
                            ("parallel", Value::Bool(true)),
                        ]),
                    ),
                    tool_call(
                        "check",
                        "read",
                        Value::object([
                            ("path", Value::string("good.txt")),
                            ("parallel", Value::Bool(false)),
                        ]),
                    ),
                ],
            ),
        ),
        (200, completion("Partial success was inspected", vec![])),
    ]);
    let mut agent = fixture_agent(&project, &home, &fixture);
    agent
        .run_turn(
            "Run independent changes and inspect their outcomes",
            &mut |_| Ok(()),
        )
        .unwrap();
    let receipts = results(&agent);
    assert!(receipts[0].1.get("bytes_written").is_some());
    assert!(receipts[1].1.get("error").is_some());
    assert!(receipts[2].1.encode().contains("good"));
    assert_eq!(
        fs::read_to_string(project.path().join("good.txt")).unwrap(),
        "good"
    );
    assert!(!project.path().join("absent.txt").exists());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn declared_groups_run_at_most_four_calls_before_starting_the_next_wave() {
    let project = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                (0..6)
                    .map(|index| write(&format!("write-{index}"), &format!("{index}.txt")))
                    .collect(),
            ),
        ),
        (200, completion("Done", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    let mut active = 0;
    let mut maximum = 0;
    let mut finished = 0;
    let mut started = Vec::new();
    agent
        .run_turn("Write six independent files", &mut |event| {
            match event {
                Event::ToolStarted { id, .. } => {
                    if id == "write-4" {
                        assert_eq!(finished, 4);
                    }
                    active += 1;
                    maximum = maximum.max(active);
                    started.push(id);
                }
                Event::ToolFinished { .. } => {
                    active -= 1;
                    finished += 1;
                }
                _ => (),
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(maximum, 4);
    assert_eq!(active, 0);
    assert_eq!(
        started,
        (0..6)
            .map(|index| format!("write-{index}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        results(&agent).iter().map(|(id, _)| id).collect::<Vec<_>>(),
        started.iter().collect::<Vec<_>>()
    );
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn cancellation_keeps_a_durable_fast_result_and_never_starts_the_dependent_call() {
    let project = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("slow.txt"), "before").unwrap();
    fs::write(project.path().join("fast.txt"), "before").unwrap();
    let fixture = HttpFixture::new(vec![(
        200,
        completion(
            "",
            vec![
                super::tests::command(
                    "slow",
                    "printf started > ready-slow; sleep 3; printf too-late > slow.txt",
                    "slow.txt",
                ),
                super::tests::command(
                    "fast",
                    "for ((n=0;n<200;n++)); do if test -e ready-slow; then printf completed > fast.txt; exit 0; fi; sleep 0.01; done; exit 73",
                    "fast.txt",
                ),
                tool_call(
                    "later",
                    "write",
                    Value::object([
                        ("path", Value::string("later.txt")),
                        ("content", Value::string("not allowed to start")),
                    ]),
                ),
            ],
        ),
    )]);
    let mut agent = fixture_agent(&project, &home, &fixture);
    agent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    let handle = agent.sessions().unwrap();
    let cancellation = agent.cancellation();
    let mut finished = Vec::new();
    let mut observed_durable = false;
    let error = agent
        .run_turn(
            "Run independent commands then dependent work",
            &mut |event| {
                if let Event::ToolFinished { id, .. } = event {
                    if id == "fast" {
                        let disk = handle.store().fixture_load(&handle.id()).unwrap();
                        assert_eq!(disk.pending.batch, ["slow", "fast"]);
                        assert_eq!(disk.pending.completed.len(), 1);
                        assert_eq!(
                            disk.pending.completed[0].get("id").and_then(Value::as_str),
                            Some("fast")
                        );
                        assert_eq!(
                            disk.pending.completed[0]
                                .get("result")
                                .unwrap()
                                .get("exit_code"),
                            Some(&Value::number(0))
                        );
                        observed_durable = true;
                        cancellation.cancel();
                    }
                    finished.push(id);
                }
                Ok(())
            },
        )
        .unwrap_err();
    assert!(error.contains("cancelled"));
    assert!(observed_durable);
    assert_eq!(finished, ["fast", "slow"]);
    assert_eq!(
        fs::read_to_string(project.path().join("fast.txt")).unwrap(),
        "completed"
    );
    assert_eq!(
        fs::read_to_string(project.path().join("slow.txt")).unwrap(),
        "before"
    );
    assert!(!project.path().join("later.txt").exists());
    let receipts = results(&agent);
    assert_eq!(
        receipts
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["slow", "fast", "later"]
    );
    assert_eq!(receipts[0].1.get("cancelled"), Some(&Value::Bool(true)));
    assert_eq!(receipts[1].1.get("exit_code"), Some(&Value::number(0)));
    assert_eq!(
        receipts[2].1.get("outcome").and_then(Value::as_str),
        Some("not_executed")
    );
    assert!(!handle.snapshot().pending.active);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn a_display_failure_preserves_returned_effects_and_prevents_later_operations() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        completion(
            "",
            vec![
                write("first", "first.txt"),
                write("second", "second.txt"),
                tool_call(
                    "later",
                    "write",
                    Value::object([
                        ("path", Value::string("later.txt")),
                        ("content", Value::string("never")),
                    ]),
                ),
            ],
        ),
    )]);
    let mut agent = fixture_agent(&project, &home, &fixture);
    let error = agent
        .run_turn("Run independent operations", &mut |event| {
            if matches!(event, Event::ToolFinished { .. }) {
                Err("display failed".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error, "display failed");
    assert!(!project.path().join("later.txt").exists());
    let receipts = results(&agent);
    assert_eq!(receipts.len(), 3);
    for (id, result) in &receipts[..2] {
        if result.get("bytes_written").is_some() {
            assert_eq!(
                fs::read_to_string(project.path().join(format!("{id}.txt"))).unwrap(),
                *id
            );
        } else {
            assert!(result.get("error").is_some());
        }
    }
    assert_eq!(
        receipts[2].1.get("outcome").and_then(Value::as_str),
        Some("not_executed")
    );
    assert!(!agent.sessions().unwrap().snapshot().pending.active);
    assert_eq!(fixture.finish().len(), 1);
}

#[cfg(windows)]
#[test]
fn an_unsaved_write_outcome_recovers_as_unknown_without_replaying_the_effect() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        completion(
            "",
            vec![
                write("first", "first.txt"),
                write("second", "second.txt"),
                tool_call(
                    "later",
                    "write",
                    Value::object([
                        ("path", Value::string("later.txt")),
                        ("content", Value::string("never")),
                    ]),
                ),
            ],
        ),
    )]);
    let mut agent = fixture_agent(&project, &home, &fixture);
    agent.prepare_turn("Run independent writes").unwrap();
    let handle = agent.sessions().unwrap();
    let store = handle.store();
    let id = handle.id();
    let bucket = fs::read_dir(home.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let journal = bucket.join(format!("{id}.jsonl"));
    let original = fs::metadata(&journal).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    let mut finishes = 0;
    let error = agent.run_turn("Run independent writes", &mut |event| {
        match event {
            Event::ToolStarted { id, .. } if id == "second" => {
                fs::set_permissions(&journal, readonly.clone()).unwrap()
            }
            Event::ToolFinished { .. } => finishes += 1,
            _ => (),
        }
        Ok(())
    });
    assert!(error.is_err());
    assert_eq!(
        finishes, 0,
        "an unsaved outcome must not be announced as complete"
    );
    let before = ["first.txt", "second.txt"].map(|path| fs::read(project.path().join(path)).ok());
    assert!(
        before.iter().any(Option::is_some),
        "at least one worker must have written before its result save failed"
    );
    assert!(!project.path().join("later.txt").exists());
    drop(agent);
    drop(handle);
    fs::set_permissions(&journal, original).unwrap();
    let (recovered, _) =
        crate::sessions::Handle::open(store, &id, crate::redact::Redactor::empty()).unwrap();
    let saved = recovered.snapshot();
    let outcomes = saved
        .messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .map(|message| {
            crate::json::parse(message.get("content").and_then(Value::as_str).unwrap()).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes
            .iter()
            .map(|result| result.get("outcome").and_then(Value::as_str))
            .collect::<Vec<_>>(),
        [Some("unknown"), Some("unknown"), Some("not_executed")]
    );
    assert_eq!(
        before,
        ["first.txt", "second.txt"].map(|path| fs::read(project.path().join(path)).ok())
    );
    assert!(!project.path().join("later.txt").exists());
    assert_eq!(fixture.finish().len(), 1);
}
