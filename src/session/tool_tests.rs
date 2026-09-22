use super::*;
use crate::workspace_fixture as support;
use crate::{
    json::{self, Value},
    providers::openai_account::{Input, Progress, Request, Response, Status, ToolCall, Usage},
    tls::{Budget, NetworkError},
    workspace::Workspace,
};
use std::{ops::ControlFlow, sync::Mutex};

#[derive(Clone, Copy)]
enum Mode {
    Batch,
    FailFollowup,
    CancelBeforeTools,
    Repeat,
    TooMany,
    MissingUsage,
}
struct Fixture {
    mode: Mode,
    requests: Arc<Mutex<Vec<String>>>,
}
impl worker::Backend for Fixture {
    fn login(
        &mut self,
        _: &Budget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn generate(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        budget.check()?;
        let mut requests = self.requests.lock().unwrap();
        requests.push(request.encode(2 * 1024 * 1024)?);
        let count = requests.len();
        drop(requests);
        if matches!(self.mode, Mode::FailFollowup) && count == 2 {
            return Err(NetworkError::Io.into());
        }
        let more_tools = count == 1 || matches!(self.mode, Mode::Repeat) && count <= 8;
        let mut response = if more_tools {
            let mut calls = vec![call("read", "read_file", r#"{"path":"notes.txt"}"#)];
            if matches!(self.mode, Mode::Batch) {
                calls.insert(0, call("list", "list_files", r#"{}"#));
                calls.push(call("search", "search_text", r#"{"query":"answer"}"#));
                calls.push(call("escape", "read_file", r#"{"path":"../outside"}"#));
            }
            if matches!(self.mode, Mode::TooMany) {
                calls = (0..33)
                    .map(|n| call(&format!("call-{n}"), "list_files", "{}"))
                    .collect();
            }
            if matches!(self.mode, Mode::CancelBeforeTools) {
                budget.cancelled.store(true, Ordering::Release);
            }
            calls_response(calls)
        } else {
            assert!(progress(Progress::Text("The answer is ")).is_continue());
            tests::response("The answer is 42.", Status::Completed)
        };
        response.usage = if matches!(self.mode, Mode::MissingUsage) && count == 2 {
            Usage::default()
        } else {
            Usage {
                input: Some(10),
                output: Some(4),
                cached: Some(2),
                reasoning: Some(1),
            }
        };
        Ok(response)
    }
}
pub(super) fn call(id: &str, name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: json::parse(arguments, Default::default()).unwrap(),
    }
}
pub(super) fn calls_response(calls: Vec<ToolCall>) -> Response {
    let mut response = tests::response("", Status::Completed);
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
    response
}
fn start(mode: Mode, enabled: bool) -> (Session, Arc<Mutex<Vec<String>>>, support::Fixture) {
    let files = support::Fixture::new();
    files.write("notes.txt", "The fixture answer is 42.\n");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let workspace = enabled.then(|| Workspace::open(&files.0).unwrap());
    let mut session = Session::with_backend(
        Model::Luna,
        Fixture {
            mode,
            requests: Arc::clone(&requests),
        },
        workspace,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    (session, requests, files)
}
fn finish(session: &mut Session) -> (End, Metrics, Vec<String>) {
    let mut events = Vec::new();
    loop {
        match tests::next(session) {
            Event::Text(text) => events.push(format!("text:{text}")),
            Event::Thinking => {}
            Event::RequestStarted => events.push("request".into()),
            Event::ToolStarted { name, .. } => events.push(format!("tool:{name}")),
            Event::ToolFinished { failed, .. } => events.push(format!("result:{failed}")),
            Event::Finished(end, metrics) => return (end, metrics, events),
            _ => panic!("unexpected event"),
        }
    }
}
pub(super) fn outputs(request: &str) -> Vec<(String, Value)> {
    let value = json::parse(request, Default::default()).unwrap();
    value
        .get("input")
        .and_then(Value::array)
        .unwrap()
        .iter()
        .filter(|item| item.get("type").and_then(Value::text) == Some("function_call_output"))
        .map(|item| {
            (
                item.get("call_id").and_then(Value::text).unwrap().into(),
                json::parse(
                    item.get("output").and_then(Value::text).unwrap(),
                    Default::default(),
                )
                .unwrap(),
            )
        })
        .collect()
}

#[test]
fn real_tools_return_ordered_results_and_usage_then_stream_a_final_answer() {
    let (mut session, requests, _files) = start(Mode::Batch, true);
    assert!(session.submit("inspect the fixture"));
    let (end, metrics, events) = finish(&mut session);
    assert_eq!(end, End::Complete);
    assert_eq!((metrics.requests, metrics.tool_calls), (2, 4));
    assert_eq!(
        (
            metrics.input_tokens,
            metrics.output_tokens,
            metrics.cached_tokens,
            metrics.reasoning_tokens
        ),
        (Some(20), Some(8), Some(4), Some(2))
    );
    assert_eq!(
        events,
        [
            "tool:list_files",
            "result:false",
            "tool:read_file",
            "result:false",
            "tool:search_text",
            "result:false",
            "tool:rejected tool",
            "result:true",
            "request",
            "text:The answer is ",
            "text:42."
        ]
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let results = outputs(&requests[1]);
    assert_eq!(
        results
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["list", "read", "search", "escape"]
    );
    assert_eq!(
        results[1].1.get("text").and_then(Value::text),
        Some("The fixture answer is 42.\n")
    );
    assert_eq!(
        results[2]
            .1
            .get("matches")
            .and_then(Value::array)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(results[3].1.get("ok"), Some(&Value::Bool(false)));
    assert!(requests[1].contains("opaque-owned-fixture"));
    drop(requests);
    drop(session); // Release native root before fixture cleanup.
}

#[test]
fn followup_failure_is_not_retried_and_explicit_next_turn_keeps_receipts() {
    let (mut session, requests, _files) = start(Mode::FailFollowup, true);
    assert!(session.submit("read the fixture"));
    let (end, metrics, _) = finish(&mut session);
    assert!(matches!(end, End::Failed(Failure::Account(_))));
    assert_eq!((metrics.requests, metrics.tool_calls), (2, 1));
    assert_eq!(metrics.input_tokens, None);
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert!(session.submit("continue explicitly"));
    let (end, metrics, _) = finish(&mut session);
    assert_eq!((end, metrics.tool_calls), (End::Complete, 0));
    let requests = requests.lock().unwrap();
    assert_eq!(outputs(&requests[2]), outputs(&requests[1]));
    drop(requests);
    drop(session);
}

#[test]
fn cancellation_after_validated_calls_prevents_reads_and_retains_unexecuted_receipts() {
    let (mut session, requests, _files) = start(Mode::CancelBeforeTools, true);
    assert!(session.submit("cancel before reads"));
    let (end, metrics, events) = finish(&mut session);
    assert_eq!(end, End::Failed(Failure::Cancelled));
    assert_eq!(metrics.tool_calls, 0);
    assert!(events.is_empty());
    assert!(session.submit("new turn"));
    assert_eq!(finish(&mut session).0, End::Complete);
    let requests = requests.lock().unwrap();
    let output = &outputs(&requests[1])[0].1;
    assert_eq!(output.get("ok"), Some(&Value::Bool(false)));
    assert!(!requests[1].contains("The fixture answer"));
    drop(requests);
    drop(session);
}

#[test]
fn bounded_loop_and_batch_do_not_execute_unbounded_work() {
    for (mode, expected_requests, expected_tools) in [(Mode::Repeat, 8, 7), (Mode::TooMany, 1, 0)] {
        let (mut session, _, _files) = start(mode, true);
        assert!(session.submit("bounded work"));
        let (end, metrics, _) = finish(&mut session);
        assert_eq!(end, End::Failed(Failure::StepLimit));
        assert_eq!(
            (metrics.requests, metrics.tool_calls),
            (expected_requests, expected_tools)
        );
        drop(session);
    }
}

#[test]
fn no_workspace_never_executes_an_unsolicited_tool_and_missing_usage_stays_unknown() {
    let (mut session, requests, _files) = start(Mode::Batch, false);
    assert!(session.submit("no workspace"));
    let (end, metrics, _) = finish(&mut session);
    assert_eq!(
        (end, metrics.tool_calls),
        (End::Failed(Failure::UnexpectedTools), 0)
    );
    assert!(
        json::parse(&requests.lock().unwrap()[0], Default::default())
            .unwrap()
            .get("tools")
            .and_then(Value::array)
            .unwrap()
            .is_empty()
    );
    drop(session);
    let (mut session, _, _files2) = start(Mode::MissingUsage, true);
    assert!(session.submit("read"));
    let (_, metrics, _) = finish(&mut session);
    assert_eq!(metrics.input_tokens, None);
    assert_eq!(metrics.output_tokens, None);
    drop(session);
}

#[test]
fn refused_or_orphaned_tool_items_never_enter_the_projection() {
    let mut history = history::History::default();
    history.begin("fixture".into()).unwrap();
    let mut response = calls_response(vec![call("read", "read_file", r#"{"path":"notes.txt"}"#)]);
    response.status = Status::Refused;
    response.tool_calls.clear();
    history.turns[0].steps.push(history::Step {
        response: Some(response),
        accepted: true,
        ..Default::default()
    });
    let request = history.request(Model::Luna, true).unwrap();
    assert!(
        request
            .input
            .iter()
            .all(|item| matches!(item, Input::User(_)))
    );
}
