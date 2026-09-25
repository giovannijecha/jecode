use super::*;
use crate::{
    providers::openai_account::{Input, Progress, Request, Response, Status, client},
    session::{End, Event, Model, Session, history::Receipt, tool_tests, worker::Backend},
    tls::Budget as NetworkBudget,
    workspace::Workspace,
};
use std::{
    ops::ControlFlow,
    sync::{Arc, Mutex, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}

pub(super) fn next_large(session: &mut Session) -> Event {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(event) = session.poll() {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "large batch worker did not produce an event"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn astra_recall_completed_prefix_of_cancelled_batch() {
    let mut history = History::default();
    history.begin("Read both files".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![
            tool_tests::call("read-a", "read_file", r#"{"path":"a.txt"}"#),
            tool_tests::call("read-b", "read_file", r#"{"path":"b.txt"}"#),
        ])),
        results: vec![
            Receipt {
                call_id: "read-a".into(),
                output: "EXACT-OLD-A".into(),
                summary: "read_file / a.txt".into(),
                image: None,
            },
            Receipt {
                call_id: "read-b".into(),
                output: String::new(),
                summary: "Not executed".into(),
                image: None,
            },
        ],
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Failed(crate::session::Failure::Cancelled));
    let cancelled = AtomicBool::new(false);
    let page = execute(&history, 0, 0, 0, 0, &budget(&cancelled));
    assert!(
        !page.failed,
        "completed receipt must survive cancellation of its sibling: {}",
        page.text
    );
    assert!(page.text.contains("EXACT-OLD-A"));
}

#[test]
fn astra_next_cursor_does_not_lead_to_excluded_effect() {
    let mut history = History::default();
    history.begin("Read, edit, read".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![
            tool_tests::call("read-a", "read_file", r#"{"path":"a.txt"}"#),
            tool_tests::call(
                "edit-a",
                "edit_file",
                r#"{"path":"a.txt","old_text":"old","new_text":"new"}"#,
            ),
            tool_tests::call("read-b", "read_file", r#"{"path":"b.txt"}"#),
        ])),
        results: vec![
            Receipt {
                call_id: "read-a".into(),
                output: "EXACT-OLD-A".into(),
                summary: "read_file / a.txt".into(),
                image: None,
            },
            Receipt {
                call_id: "edit-a".into(),
                output: "applied".into(),
                summary: "edit_file / applied".into(),
                image: None,
            },
            Receipt {
                call_id: "read-b".into(),
                output: "EXACT-OLD-B".into(),
                summary: "read_file / b.txt".into(),
                image: None,
            },
        ],
        accepted: true,
        ..Default::default()
    });
    let cancelled = AtomicBool::new(false);
    let first = execute(&history, 0, 0, 0, 0, &budget(&cancelled));
    assert!(!first.failed);
    let value = json::parse(&first.text, Default::default()).unwrap();
    let next = value.get("next").unwrap();
    let receipt = next.get("receipt").and_then(Value::unsigned).unwrap() as usize;
    let offset = next.get("offset").and_then(Value::unsigned).unwrap() as usize;
    let second = execute(&history, 0, 0, receipt, offset, &budget(&cancelled));
    assert!(
        !second.failed,
        "following the emitted next cursor failed: {}",
        second.text
    );
    assert!(second.text.contains("EXACT-OLD-B"));
}

struct LargeRecallBatch {
    round: usize,
    admitted: Arc<Mutex<Option<(usize, usize, usize)>>>,
}
impl Backend for LargeRecallBatch {
    fn login(
        &mut self,
        _: &NetworkBudget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn generate(
        &mut self,
        request: &Request,
        _: &NetworkBudget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        if request.instructions.starts_with("Summarize") {
            return Ok(crate::session::tests::handoff_response(
                request,
                "Original read is saved at turn 0 step 0.",
            ));
        }
        self.round += 1;
        if self.round == 1 {
            let calls = (0..1100)
                .map(|n| {
                    tool_tests::call(
                        &format!("recall-{n}"),
                        "recall_receipts",
                        r#"{"turn":0,"step":0}"#,
                    )
                })
                .collect();
            let response = tool_tests::calls_response(calls);
            assert!(json::encode(&Value::Array(response.output.clone()), 1024 * 1024).is_ok());
            return Ok(response);
        }
        if self.round == 2 {
            let mut count = 0;
            let mut successful = 0;
            let mut rejected = 0;
            for item in &request.input {
                if let Input::ToolResult { call_id, output } = item
                    && call_id.starts_with("recall-")
                {
                    assert_eq!(call_id, &format!("recall-{count}"));
                    let value = json::parse(output, Default::default()).unwrap();
                    if value.get("ok") == Some(&Value::Bool(true)) {
                        assert!(output.contains(&"x".repeat(8000)));
                        successful += 1;
                    } else {
                        assert!(output.contains("aggregate") || output.contains("not_executed"));
                        rejected += 1;
                    }
                    count += 1;
                }
            }
            assert!(request.encode(super::super::history::MAX_REQUEST).is_ok());
            *self.admitted.lock().unwrap() = Some((count, successful, rejected));
        }
        Ok(crate::session::tests::response(
            "Consumed the admitted receipt pages",
            Status::Completed,
        ))
    }
}

#[test]
fn astra_legal_recall_batch_does_not_permanently_block_continuation() {
    let files = crate::workspace_fixture::Fixture::new();
    let mut history = History::default();
    history.begin("Read original evidence".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "original",
            "read_file",
            r#"{"path":"evidence.txt"}"#,
        )])),
        results: vec![Receipt {
            call_id: "original".into(),
            output: "x".repeat(8000),
            summary: "read_file / evidence.txt".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    let admitted = Arc::new(Mutex::new(None));
    let mut session = Session::with_history(
        Model::Luna,
        LargeRecallBatch {
            round: 0,
            admitted: admitted.clone(),
        },
        Some(Workspace::open(&files.0).unwrap()),
        history,
    )
    .unwrap();
    assert!(matches!(next_large(&mut session), Event::Ready));
    let mut outcomes = Vec::new();
    for prompt in ["Recover the saved evidence", "Continue"] {
        assert!(session.submit(prompt));
        loop {
            if let Event::Finished(end, metrics) = next_large(&mut session) {
                eprintln!(
                    "legal recall batch outcome: {end:?}, requests={}, tools={}",
                    metrics.requests, metrics.tool_calls
                );
                outcomes.push(end);
                break;
            }
        }
    }
    assert_eq!(
        outcomes,
        vec![End::Complete, End::Complete],
        "individually valid recall results must not create an unrecoverable request"
    );
    let (count, successful, rejected) = admitted.lock().unwrap().unwrap();
    assert_eq!(count, 1100);
    assert!(successful > 0 && rejected > 0);
}
