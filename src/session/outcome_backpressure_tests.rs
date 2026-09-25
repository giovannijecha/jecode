//! Saturate presentation while the real controller persists and executes effects.
use super::*;
use crate::{
    command::tests as native,
    json::{self, Value},
    providers::openai_account::{Progress, Request, Response, Status},
    tls::Budget,
    workspace_fixture::Fixture as Files,
};
use std::{
    fs,
    ops::ControlFlow,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Case {
    Edit,
    Command,
    TwoEdits,
    FailedEdit,
}

struct Backend {
    case: Case,
    round: usize,
}
impl worker::Backend for Backend {
    fn login(
        &mut self,
        _: &Budget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn generate(
        &mut self,
        _: &Request,
        _: &Budget<'_>,
        progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.round += 1;
        if self.round != 1 {
            return Ok(tests::response("Recorded the result.", Status::Completed));
        }
        for _ in 0..16 {
            let _ = progress(Progress::Text("x"));
        }
        let mut calls = match self.case {
            Case::Command => {
                let arguments = json::encode(
                    &json::object([
                        ("command", Value::String(native::script("write"))),
                        ("timeout_seconds", Value::Number("90".into())),
                    ]),
                    8192,
                )
                .unwrap();
                vec![tool_tests::call("one", "run_command", &arguments)]
            }
            _ => vec![tool_tests::call(
                "one",
                "edit_file",
                r#"{"path":"notes.txt","old_text":"old","new_text":"new"}"#,
            )],
        };
        if matches!(self.case, Case::TwoEdits) {
            calls.push(tool_tests::call(
                "two",
                "create_file",
                r#"{"path":"created.txt","content":"created\n"}"#,
            ));
        }
        let mut response = tests::response(&"x".repeat(16), Status::Completed);
        for call in &calls {
            response.output.push(json::object([
                ("type", Value::String("function_call".into())),
                ("call_id", Value::String(call.id.clone())),
                ("name", Value::String(call.name.clone())),
                (
                    "arguments",
                    Value::String(json::encode(&call.arguments, 4096).unwrap()),
                ),
            ]));
        }
        response.tool_calls = calls;
        Ok(response)
    }
}

fn saved_receipts(store: &crate::state::Store, id: &str) -> Option<Vec<Value>> {
    let saved = persistence::load(store, id, false).ok()?;
    let results = &saved.history.turns.last()?.steps.first()?.results;
    results
        .iter()
        .map(|receipt| json::parse(&receipt.output, Default::default()).ok())
        .collect()
}

pub(crate) fn saturated_events(case: Case) -> Option<Vec<Event>> {
    let home = crate::state::tests::Fixture::new();
    let store = home.store()?;
    let files = Files::new();
    files.write("notes.txt", "old\n");
    let workspace = crate::workspace::Workspace::open(&files.0).unwrap();
    let mut history =
        persistence::create_in(&store, Model::Luna, Some(&files.0), Some(&workspace)).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    let target = files.0.join("notes.txt");
    let gate_entered = Arc::new(AtomicBool::new(false));
    let checkpointed = Arc::new(AtomicBool::new(false));
    history.test_outcome_checkpoint = Some(Arc::clone(&checkpointed));
    let reached = Arc::clone(&gate_entered);
    history.effect_gate = Some(Arc::new(move |name, events| {
        reached.store(true, Ordering::Release);
        if matches!(case, Case::FailedEdit) && name == "edit_file" {
            fs::write(&target, "competing\n").unwrap();
        }
        // The plan has been delivered. Fill the presentation channel without
        // adding visible text, then let the real effect and checkpoint proceed.
        while events.try_send(Event::Text(String::new())).is_ok() {}
    }));
    let mut session = Session::with_history(
        Model::Luna,
        Backend { case, round: 0 },
        Some(workspace),
        history,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Restored { .. }));
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.submit("run isolated effects"));

    // The test gate fills all 64 slots after the plan, so the effect can start
    // without depending on the number of setup events. Wait for its committed
    // receipt without draining presentation.
    let expected = if matches!(case, Case::Command) {
        "exited"
    } else if matches!(case, Case::FailedEdit) {
        "failed"
    } else {
        "applied"
    };
    // PowerShell startup can queue behind other native process tests on CI.
    // Its command deadline is longer than this observation window.
    let deadline = Instant::now()
        + Duration::from_secs(if matches!(case, Case::Command) {
            60
        } else {
            35
        });
    while !checkpointed.load(Ordering::Acquire) {
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // The channel is full, so the worker is blocked delivering completion
    // after the successful checkpoint. Read the committed head only once.
    let before_release = if checkpointed.load(Ordering::Acquire) {
        saved_receipts(&store, &id)
    } else {
        None
    };
    let mut events = Vec::new();
    let finish_deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(event) = session.poll() {
            let finished = matches!(event, Event::Finished(..));
            events.push(event);
            if finished {
                break;
            }
        } else if Instant::now() >= finish_deadline {
            break;
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    drop(session);
    let final_receipts = saved_receipts(&store, &id);
    assert!(
        before_release.is_some(),
        "{case:?} receipt did not reach storage before presentation drained; gate={}, checkpoint={}, events={}, last={:?}, final={final_receipts:?}",
        gate_entered.load(Ordering::Acquire),
        checkpointed.load(Ordering::Acquire),
        events.len(),
        events.last().map(std::mem::discriminant)
    );
    assert_eq!(
        before_release.unwrap()[0]
            .get("status")
            .and_then(Value::text),
        Some(expected)
    );
    assert!(matches!(
        events.last(),
        Some(Event::Finished(End::Complete, _))
    ));
    assert!(events.len() >= 64, "presentation was not saturated");
    let final_receipts = final_receipts.unwrap();
    assert_eq!(
        final_receipts[0].get("status").and_then(Value::text),
        Some(expected)
    );
    match case {
        Case::Edit | Case::TwoEdits => assert_eq!(
            fs::read_to_string(files.0.join("notes.txt")).unwrap(),
            "new\n"
        ),
        Case::Command => {
            assert_eq!(
                fs::read_to_string(files.0.join("command-result.txt")).unwrap(),
                "one execution\n"
            );
            assert_eq!(final_receipts[0].get("executed"), Some(&Value::Bool(true)));
            assert_eq!(final_receipts[0].get("truncated"), Some(&Value::Bool(true)));
        }
        Case::FailedEdit => assert_eq!(
            fs::read_to_string(files.0.join("notes.txt")).unwrap(),
            "competing\n"
        ),
    }
    if matches!(case, Case::TwoEdits) {
        assert_eq!(
            fs::read_to_string(files.0.join("created.txt")).unwrap(),
            "created\n"
        );
        assert_eq!(
            final_receipts[1].get("status").and_then(Value::text),
            Some("applied")
        );
    }
    Some(events)
}

#[test]
fn completed_effects_reach_durable_receipts_before_saturated_presentation_drains() {
    for case in [Case::Edit, Case::Command, Case::TwoEdits, Case::FailedEdit] {
        let Some(events) = saturated_events(case) else {
            return;
        };
        let operations = events
            .iter()
            .filter_map(|event| match event {
                Event::EditPlanned { id, .. } | Event::CommandPlanned { id, .. } => {
                    Some((*id, "planned"))
                }
                Event::EditFinished { id, .. } | Event::CommandFinished { id, .. } => {
                    Some((*id, "finished"))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let expected = if matches!(case, Case::TwoEdits) {
            vec![
                (1, "planned"),
                (1, "finished"),
                (2, "planned"),
                (2, "finished"),
            ]
        } else {
            vec![(1, "planned"), (1, "finished")]
        };
        assert_eq!(operations, expected);
    }
}
