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
    Pressure,
    TooMany,
    MissingUsage,
}
struct Fixture {
    mode: Mode,
    requests: Arc<Mutex<Vec<String>>>,
    round: usize,
    deadline_min: Option<std::time::Instant>,
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
        if let Some(minimum) = self.deadline_min {
            assert!(
                budget.deadline >= minimum,
                "generation inherited the old task deadline"
            );
        }
        budget.check()?;
        let mut requests = self.requests.lock().unwrap();
        requests.push(request.encode(2 * 1024 * 1024)?);
        let summary = request.instructions.starts_with("Summarize");
        drop(requests);
        if summary {
            return Ok(tests::response(
                "Keep the user's original task, guidance and completed read receipts.",
                Status::Completed,
            ));
        }
        self.round += 1;
        let count = self.round;
        if matches!(self.mode, Mode::FailFollowup) && count == 2 {
            return Err(NetworkError::io(
                crate::tls::IoOperation::ReadRecordHeader,
                &std::io::Error::from(std::io::ErrorKind::ConnectionReset),
            )
            .into());
        }
        let more_tools =
            count == 1 || matches!(self.mode, Mode::Repeat | Mode::Pressure) && count <= 40;
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
            let mut response = calls_response(calls);
            if matches!(self.mode, Mode::Pressure) {
                let large = "x".repeat(8192);
                let message = tests::response(&large, Status::Completed);
                response.text = large;
                response.output.splice(0..0, message.output);
            }
            response
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
            round: 0,
            deadline_min: None,
        },
        workspace,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    (session, requests, files)
}
#[test]
fn selected_pair_is_used_for_every_request_in_a_tool_loop() {
    let (mut session, requests, _files) = start(Mode::Batch, true);
    let selected = Model::new("account-model", Some("xhigh")).unwrap();
    assert!(session.set_model(selected));
    assert!(matches!(tests::next(&mut session), Event::ModelChanged(value) if value == selected));
    assert!(session.submit("inspect the fixture"));
    assert_eq!(finish(&mut session).0, End::Complete);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        let value = json::parse(request, Default::default()).unwrap();
        assert_eq!(
            value.get("model").and_then(Value::text),
            Some("account-model")
        );
        assert_eq!(
            value
                .get("reasoning")
                .and_then(|v| v.get("effort"))
                .and_then(Value::text),
            Some("xhigh")
        );
    }
    drop(requests);
    drop(session);
}
fn finish(session: &mut Session) -> (End, Metrics, Vec<String>) {
    finish_with(session, tests::next)
}
fn finish_with(
    session: &mut Session,
    mut next: impl FnMut(&mut Session) -> Event,
) -> (End, Metrics, Vec<String>) {
    let mut events = Vec::new();
    loop {
        match next(session) {
            Event::Text(text) => events.push(format!("text:{text}")),
            Event::Thinking => {}
            Event::RequestStarted => events.push("request".into()),
            Event::ContextReport(_) => {}
            Event::ToolStarted { name, .. } => events.push(format!("tool:{name}")),
            Event::ToolFinished { failed, .. } => events.push(format!("result:{failed}")),
            Event::Finished(end, metrics) => return (end, metrics, events),
            _ => panic!("unexpected event"),
        }
    }
}
fn next_large_batch(session: &mut Session) -> Event {
    // This fixture serializes and checks several near-2-MiB requests in a
    // debug build. Bound its wait independently of the small fixture helper.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(event) = session.poll() {
            return event;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "large batch worker did not produce an event"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
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
            // Syntax is valid; the active workspace profile rejects access.
            "tool:read_file",
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
fn loop_and_batch_continue_past_former_count_limits() {
    for (mode, expected_requests, expected_tools) in
        [(Mode::Repeat, 41, 40), (Mode::TooMany, 2, 33)]
    {
        let (mut session, _, _files) = start(mode, true);
        assert!(session.submit("bounded work"));
        let (end, metrics, _) = finish(&mut session);
        assert_eq!(end, End::Complete);
        assert_eq!(
            (metrics.requests, metrics.tool_calls),
            (expected_requests, expected_tools)
        );
        drop(session);
    }
}

#[test]
fn first_turn_compacts_repeatedly_after_former_accumulated_text_limit() {
    let files = support::Fixture::new();
    files.write("notes.txt", "The fixture answer is 42.\n");
    let workspace = Workspace::open(&files.0).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut history = history::History::default();
    history.projection.limit_bytes = 65536;
    let mut session = Session::with_history(
        Model::Luna,
        Fixture {
            mode: Mode::Pressure,
            requests: Arc::clone(&requests),
            round: 0,
            deadline_min: None,
        },
        Some(workspace),
        history,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.submit("Complete the original fixture task"));
    let (end, metrics, _) = finish(&mut session);
    assert_eq!(end, End::Complete);
    assert_eq!(metrics.tool_calls, 40);
    let requests = requests.lock().unwrap();
    let summaries = requests
        .iter()
        .filter(|r| r.contains("Summarize this bounded portion"))
        .count();
    let generations: Vec<_> = requests
        .iter()
        .filter(|r| !r.contains("Summarize this bounded portion"))
        .collect();
    assert!(summaries >= 2, "compactions={summaries}");
    assert_eq!(generations.len(), 41);
    assert_eq!(metrics.requests as usize, generations.len() + summaries);
    assert!(generations.iter().all(|r| r.len() <= 65536));
    assert!(
        generations
            .last()
            .unwrap()
            .contains("original fixture task")
    );
    for request in &generations {
        let value = json::parse(request, Default::default()).unwrap();
        let input = value.get("input").and_then(Value::array).unwrap();
        let calls = input
            .iter()
            .filter(|item| item.get("type").and_then(Value::text) == Some("function_call"))
            .count();
        let receipts = input
            .iter()
            .filter(|item| item.get("type").and_then(Value::text) == Some("function_call_output"))
            .count();
        assert_eq!(calls, receipts, "unpaired projected tool call");
    }
    eprintln!(
        "pressure fixture: requests={} tools={} compactions={} max_projected_bytes={}",
        metrics.requests,
        metrics.tool_calls,
        summaries,
        generations.iter().map(|r| r.len()).max().unwrap()
    );
    drop(requests);
    drop(session);
}

#[test]
fn old_task_age_does_not_expire_later_operation_deadlines() {
    let files = support::Fixture::new();
    files.write("notes.txt", "The fixture answer is 42.\n");
    let workspace = Workspace::open(&files.0).unwrap();
    let (events, _received) = std::sync::mpsc::sync_channel(64);
    let (_decision, decisions) = std::sync::mpsc::sync_channel(1);
    let context = worker::Context {
        events,
        cancelled: Arc::new(AtomicBool::new(false)),
        stopped: Arc::new(AtomicBool::new(false)),
        decisions,
        guidance: Arc::new(queue::Pending::default()),
        next_approval: std::sync::atomic::AtomicU64::new(1),
    };
    let mut history = history::History::default();
    history.begin("Task older than ten minutes".into()).unwrap();
    let started = std::time::Instant::now();
    let later = started + std::time::Duration::from_secs(700);
    let mut backend = Fixture {
        mode: Mode::Batch,
        requests: Arc::new(Mutex::new(Vec::new())),
        round: 0,
        deadline_min: Some(later + std::time::Duration::from_secs(600)),
    };
    let mut metrics = Metrics::default();
    let clock_calls = std::cell::Cell::new(0);
    assert_eq!(
        tool_loop::run(
            &mut backend,
            &mut history,
            &context,
            Model::Luna,
            Some(&workspace),
            started,
            || {
                clock_calls.set(clock_calls.get() + 1);
                later
            },
            &mut metrics
        ),
        Ok(End::Complete)
    );
    assert_eq!((metrics.requests, metrics.tool_calls), (2, 4));
    assert_eq!(clock_calls.get(), 6); // Two generations and four reads.
    drop(workspace);
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
#[test]
fn large_completed_batch_is_compacted_as_bounded_reference_data() {
    use std::collections::{BTreeMap, BTreeSet};
    struct LargeBatch {
        requests: Arc<Mutex<Vec<(bool, String)>>>,
        seen: BTreeSet<String>,
        fragments: BTreeMap<usize, (usize, String)>,
        generation: usize,
    }
    impl worker::Backend for LargeBatch {
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
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            budget.check()?;
            let encoded = request.encode(history::MAX_CONTEXT)?;
            let summary = request.instructions.starts_with("Summarize");
            self.requests
                .lock()
                .unwrap()
                .push((summary, encoded.clone()));
            if summary {
                assert!(request.tools.is_empty());
                let value = json::parse(&encoded, Default::default()).unwrap();
                let input = value.get("input").and_then(Value::array).unwrap();
                assert!(input.iter().all(|item| item.get("type").and_then(Value::text) != Some("function_call")));
                assert!(
                    input
                        .iter()
                        .all(|item| item.get("type").and_then(Value::text)
                            != Some("function_call_output"))
                );
                for item in &request.input {
                    if let Input::User(data) = item
                        && data.starts_with("Completed step record ")
                    {
                        let (header, fragment) = data.split_once('\n').unwrap();
                        let record_index: usize = header
                            .strip_prefix("Completed step record ")
                            .unwrap()
                            .split(',')
                            .next()
                            .unwrap()
                            .parse()
                            .unwrap();
                        let range = header
                            .split("bytes ")
                            .nth(1)
                            .unwrap()
                            .split(';')
                            .next()
                            .unwrap();
                        let (start, rest) = range.split_once("..").unwrap();
                        let (end, total) = rest.split_once(" of ").unwrap();
                        let (start, end, total): (usize, usize, usize) = (
                            start.parse().unwrap(),
                            end.parse().unwrap(),
                            total.parse().unwrap(),
                        );
                        let entry = self
                            .fragments
                            .entry(record_index)
                            .or_insert_with(|| (total, String::new()));
                        assert_eq!(entry.0, total);
                        assert_eq!(entry.1.len(), start);
                        entry.1.push_str(fragment);
                        assert_eq!(entry.1.len(), end);
                        if end != total {
                            continue;
                        }
                        let (_, complete) = self.fragments.remove(&record_index).unwrap();
                        let record = json::parse(&complete, Default::default()).unwrap();
                        if record.get("kind").and_then(Value::text)
                            == Some("response_item_and_receipt")
                            && record.get("call_id").and_then(Value::text).is_some()
                        {
                            let call_id = record.get("call_id").and_then(Value::text).unwrap();
                            let n: usize = call_id.strip_prefix("read-").unwrap().parse().unwrap();
                            assert!(n < 100);
                            assert_eq!(
                                record.get("call_name").and_then(Value::text),
                                Some("read_file")
                            );
                            assert_eq!(
                                record.get("receipt_call_id").and_then(Value::text),
                                Some(call_id)
                            );
                            assert_eq!(
                                record
                                    .get("parsed_arguments")
                                    .and_then(|v| v.get("path"))
                                    .and_then(Value::text),
                                Some(format!("part-{n:03}.txt").as_str())
                            );
                            let marker = format!("MARKER-{n:03}");
                            assert!(
                                record
                                    .get("receipt_output")
                                    .and_then(Value::text)
                                    .unwrap()
                                    .contains(&marker)
                            );
                            assert!(
                                self.seen.insert(marker),
                                "duplicate canonical result in summary slices"
                            );
                        }
                    }
                }
                return Ok(tests::response(
                    &format!(
                        "Completed read results: {}",
                        self.seen.iter().cloned().collect::<Vec<_>>().join(" ")
                    ),
                    Status::Completed,
                ));
            }
            self.generation += 1;
            if self.generation == 1 {
                let calls = (0..100)
                    .map(|n| {
                        call(
                            &format!("read-{n:03}"),
                            "read_file",
                            &format!(r#"{{"path":"part-{n:03}.txt"}}"#),
                        )
                    })
                    .collect();
                return Ok(calls_response(calls));
            }
            assert_eq!(self.seen.len(), 100, "summary omitted completed results");
            assert!(self.fragments.is_empty());
            for n in 0..100 {
                assert!(encoded.contains(&format!("MARKER-{n:03}")));
            }
            Ok(tests::response(
                "Verified all 100 completed reads.",
                Status::Completed,
            ))
        }
    }
    let files = support::Fixture::new();
    for n in 0..100 {
        files.write(
            &format!("part-{n:03}.txt"),
            format!("MARKER-{n:03}{}\n", "\t".repeat(8000)),
        );
    }
    let workspace = Workspace::open(&files.0).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_backend(
        Model::Luna,
        LargeBatch {
            requests: requests.clone(),
            seen: BTreeSet::new(),
            fragments: BTreeMap::new(),
            generation: 0,
        },
        Some(workspace),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.submit("Read every distinct part and verify all 100 markers"));
    let (end, metrics, _) = finish_with(&mut session, next_large_batch);
    assert_eq!(end, End::Complete);
    assert_eq!(metrics.tool_calls, 100);
    let requests = requests.lock().unwrap();
    let compactions = requests.iter().filter(|(summary, _)| *summary).count();
    assert!(compactions >= 2);
    assert_eq!(metrics.requests as usize, requests.len());
    assert_eq!(requests.iter().filter(|(summary, _)| !summary).count(), 2);
    assert!(
        requests
            .iter()
            .all(|(_, encoded)| encoded.len() <= history::MAX_CONTEXT)
    );
    eprintln!(
        "large batch fixture: requests={} tools={} compactions={} max_projected_bytes={}",
        metrics.requests,
        metrics.tool_calls,
        compactions,
        requests
            .iter()
            .map(|(_, encoded)| encoded.len())
            .max()
            .unwrap()
    );
    drop(requests);
    drop(session);
}
