use super::*;
use crate::context::{AUTO_COMPACT_DEFAULT, Status, threshold};
use crate::test_support::{Directory, HttpFixture, completion, tool_call};

/// Context projections in order, with the turn events that explain them.
fn trace(agent: &mut Agent, prompt: &str) -> (Result<(), String>, Vec<(&'static str, Status)>) {
    let mut trace = Vec::new();
    let mut last = None;
    let result = agent.run_turn(prompt, &mut |event| {
        let label = match event {
            Event::Recovering { .. } => return Err("Unexpected fixture request".into()),
            Event::Context(status) => {
                last = Some(status);
                "context"
            }
            Event::RequestDiscarded => "discarded",
            Event::ContextCompacted { .. } => "compacted",
            Event::Maintenance { .. } => "maintenance",
            _ => return Ok(()),
        };
        if let Some(status) = last {
            trace.push((label, status));
        }
        Ok(())
    });
    (result, trace)
}

fn assistant(text: String) -> Value {
    Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string(text)),
    ])
}

#[test]
fn new_and_resumed_conversations_use_the_current_preference() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Done.", vec![]))]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.prepare_turn("First").unwrap();
    agent.run_turn("First", &mut |_| Ok(())).unwrap();
    let first = agent.sessions().unwrap().id();
    agent.set_auto_compact(30);
    agent
        .start_new("fixture/model".into(), agent.effort())
        .unwrap();
    assert_eq!(agent.context_status().auto_compact, 30);
    agent.set_auto_compact(45);
    agent.resume(&first).unwrap();
    assert_eq!(agent.context_status().auto_compact, 45);
    // The preference is global, so saved conversations never carry it.
    fn mentions(path: &std::path::Path) -> bool {
        if path.is_dir() {
            return std::fs::read_dir(path)
                .unwrap()
                .any(|entry| mentions(&entry.unwrap().path()));
        }
        std::fs::read(path).is_ok_and(|bytes| {
            bytes
                .windows(b"auto_compact".len())
                .any(|window| window == b"auto_compact")
        })
    }
    assert!(!mentions(&home.path().join("sessions")));
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn unknown_capacity_stays_unknown_and_a_model_switch_forgets_the_previous_window() {
    let directory = Directory::new();
    let client = OpenRouter::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    let status = agent.context_status();
    assert!(status.tokens > 0);
    assert_eq!(status.capacity, None);
    assert_eq!(status.percent(), None);
    assert_eq!(status.auto_compact, AUTO_COMPACT_DEFAULT);
    agent.client.fixture_limits(24000, Some(4096));
    agent.context.ceiling = Some(12000);
    assert_eq!(agent.context_status().capacity, Some(12000));
    agent.set_model("fixture/other".into()).unwrap();
    assert_eq!(agent.context_status().capacity, None);
}

#[test]
fn preview_only_reduction_reports_the_reduced_projection() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Reported.", vec![]))]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_limits(24000, None);
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent.prepare_turn("Inspect once and report.").unwrap();
    let output = format!("HEAD\n{}\nTAIL", "x".repeat(80000));
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "once",
                    "bash",
                    Value::object([("command", Value::string("inspect-once"))]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("once")),
            (
                "content",
                Value::string(
                    Value::object([
                        ("exit_code", Value::number(0)),
                        ("stdout", Value::string(&output)),
                        ("stderr", Value::string("")),
                    ])
                    .encode(),
                ),
            ),
        ]),
    ]);
    let original = agent.messages.lock().unwrap().clone();
    agent.context.observe(
        Some(&crate::backend::Usage {
            input_tokens: Some(6000),
            ..Default::default()
        }),
        2,
    );
    agent.context.calibrate(36000);
    let (result, trace) = trace(&mut agent, "Inspect once and report.");
    result.unwrap();
    let budget = threshold(24000, AUTO_COMPACT_DEFAULT);
    let (_, before) = trace[0];
    assert_eq!(before.capacity, Some(24000));
    assert!(before.tokens >= budget);
    assert!(before.percent().unwrap() >= usize::from(AUTO_COMPACT_DEFAULT));
    let reduced = trace
        .iter()
        .skip_while(|(label, _)| *label != "maintenance")
        .find(|(label, _)| *label == "context")
        .unwrap()
        .1;
    assert!(reduced.tokens > 0 && reduced.tokens < budget);
    assert!(trace.iter().all(|(label, _)| *label != "compacted"));
    assert_eq!(agent.messages.lock().unwrap()[..original.len()], original);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn a_summary_reports_the_reduced_projection_instead_of_zero() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("Goal kept; next action: finish.", vec![])),
        (200, completion("Goal kept; next action: finish.", vec![])),
        (200, completion("Finished.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.client.fixture_limits(32000, Some(4096));
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Start the long task.")),
        ]),
        assistant("a".repeat(12000)),
        assistant("b".repeat(12000)),
        assistant("c".repeat(12000)),
    ]);
    let original = agent.messages.lock().unwrap().clone();
    let (result, trace) = trace(&mut agent, "Continue.");
    result.unwrap();
    let budget = threshold(32000, AUTO_COMPACT_DEFAULT);
    assert!(trace[0].1.tokens >= budget);
    let compacted = trace.iter().position(|(label, _)| *label == "compacted");
    let reduced = trace[compacted.unwrap()..]
        .iter()
        .find(|(label, _)| *label == "context")
        .unwrap()
        .1;
    assert!(reduced.tokens > 0 && reduced.tokens < budget);
    assert_eq!(reduced.capacity, Some(32000));
    assert_eq!(agent.messages.lock().unwrap()[..original.len()], original);
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn provider_recovery_reports_the_learned_ceiling_when_capacity_was_unknown() {
    let directory = Directory::new();
    let prompt = "original objective ".repeat(1500);
    let fixture = HttpFixture::new(vec![
        (
            400,
            Value::object([(
                "error",
                Value::object([
                    ("code", Value::string("context_length_exceeded")),
                    ("message", Value::string("Maximum context length exceeded")),
                ]),
            )]),
        ),
        (200, completion("Original objective retained.", vec![])),
        (200, completion("Original objective retained.", vec![])),
        (200, completion("Finished.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.client.fixture_unknown_limits();
    let (result, trace) = trace(&mut agent, &prompt);
    result.unwrap();
    let discarded = trace
        .iter()
        .position(|(label, _)| *label == "discarded")
        .unwrap();
    assert!(
        trace[..discarded]
            .iter()
            .all(|(_, status)| status.capacity.is_none() && status.percent().is_none())
    );
    let ceiling = agent.context.ceiling.unwrap();
    let learned = trace[discarded + 1..]
        .iter()
        .find(|(label, _)| *label == "context")
        .unwrap()
        .1;
    assert_eq!(learned.capacity, Some(ceiling));
    assert!(trace.iter().any(|(label, _)| *label == "compacted"));
    assert_eq!(fixture.finish().len(), 4);
}

#[test]
fn an_unattainable_threshold_fails_once_and_preserves_history() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.client.fixture_limits(24000, Some(4096));
    agent.set_auto_compact(1);
    let from = agent.context.from;
    let (result, trace) = trace(&mut agent, "Keep this request.");
    let error = result.unwrap_err();
    assert!(error.contains("1% of 24000 tokens"), "{error}");
    assert!(error.contains("Auto-compact at"));
    assert_eq!(agent.context.from, from);
    assert!(agent.context.summary.is_empty());
    let messages = agent.messages.lock().unwrap();
    assert_eq!(
        messages[1].get("content").and_then(Value::as_str),
        Some("Keep this request.")
    );
    assert!(trace.iter().all(|(label, _)| *label == "context"));
    drop(messages);
    assert!(fixture.finish().is_empty());
}

#[test]
fn accepted_input_updates_context_before_a_summary_starts() {
    for with_tool in [false, true] {
        let directory = Directory::new();
        let workspace = directory
            .path()
            .join("local-runner/jobs/jecode/fresh-checkout/project");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("source"), "Original evidence.").unwrap();
        let calls = if with_tool {
            vec![tool_call(
                "inspect",
                "read",
                Value::object([("path", Value::string("source"))]),
            )]
        } else {
            vec![]
        };
        let fixture = HttpFixture::new(vec![
            (200, completion("First result.", calls)),
            (
                200,
                completion("Continue with the updated requirement.", vec![]),
            ),
            (200, completion("Finished.", vec![])),
        ]);
        let mut client = OpenRouter::fixture(fixture.endpoint.clone());
        client.fixture_limits(32000, Some(4096));
        let mut agent = Agent::new(client, Tools::new(&workspace).unwrap());
        let inbox = agent.inbox();
        let update = "Updated requirement. ".repeat(800);
        let mut submitted = false;
        let mut accepted = false;
        let mut accepted_context = None;
        let mut before_summary = None;
        let mut summaries_started = 0;
        agent
            .run_turn("Start the task.", &mut |event| {
                let submit = if with_tool {
                    matches!(&event, Event::ToolFinished { .. })
                } else {
                    matches!(&event, Event::Message { .. })
                };
                if submit && !submitted {
                    inbox.update(|entries| {
                        entries.push(update.clone().into()).unwrap();
                        entries.record();
                    });
                    submitted = true;
                }
                match event {
                    Event::Context(status) if accepted => accepted_context = Some(status),
                    Event::Accepted { .. } => accepted = true,
                    Event::Maintenance { text } if text.starts_with("Compacting context") => {
                        summaries_started += 1;
                        if accepted {
                            before_summary = accepted_context;
                        }
                    }
                    _ => {}
                }
                Ok(())
            })
            .unwrap();
        let status = before_summary.expect("Accepted input should trigger a summary");
        assert_eq!(summaries_started, 1);
        assert!(
            status.tokens >= threshold(32000, AUTO_COMPACT_DEFAULT),
            "The footer must show accepted input before summarizing it (with_tool={with_tool}): {status:?}"
        );
        assert_eq!(fixture.finish().len(), 3);
    }
}

#[test]
fn retired_view_recalculates_with_current_system_and_request_environment() {
    use crate::attachments::{MARKER, Prompt};

    let project = Directory::new();
    let home = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.project_instructions = "A changed project instruction".into();
    let mut image = crate::attachments::tests::attachment("att-1-1-1", "shot.png", "image/png");
    image.width = Some(4);
    image.height = Some(3);
    let receipt = Value::object([
        ("view", Value::string("image")),
        ("attachment", image.value()),
    ]);
    let measured_end = {
        let mut messages = agent.messages.lock().unwrap();
        messages.push(Prompt::new(MARKER.to_string(), vec![image]).message());
        messages.push(Value::object([
            ("role", Value::string("tool")),
            ("content", Value::string(receipt.encode())),
        ]));
        messages.len()
    };
    agent.context.observe(
        Some(&crate::backend::Usage {
            input_tokens: Some(50_000),
            ..Default::default()
        }),
        measured_end,
    );
    agent.context.calibrate(1000);
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("I saw it")),
    ]));
    let messages = agent.messages.lock().unwrap();
    assert_eq!(agent.context.measured_input(&messages), None);
    assert!(!agent.request_environment().is_empty());
    let projected = agent.projected_context(&agent.context, &messages);
    assert!(
        projected[0]
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("A changed project instruction")
    );
    let expected = agent
        .context
        .estimate_bytes(agent.context_bytes(&projected) + crate::tools::encoded_size())
        + agent.request_environment().len();
    assert_eq!(agent.context_estimate(&messages), expected);
}
