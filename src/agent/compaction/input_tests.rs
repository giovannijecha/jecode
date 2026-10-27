use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::inbox::Inbox;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use crate::tools::Tools;

fn agent(directory: &Directory, fixture: &HttpFixture, limits: Limits) -> Agent {
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_limits(limits.context, limits.output);
    Agent::new(client, Tools::new(directory.path()).unwrap())
}

fn send(inbox: &Inbox, text: &str) {
    inbox.update(|entries| {
        entries.push(text.into()).unwrap();
        entries.record();
    });
}

#[test]
fn input_during_a_tool_preview_joins_the_immediately_following_request() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Finished.", vec![]))]);
    let limits = Limits {
        context: 24000,
        output: None,
    };
    let mut agent = agent(&directory, &fixture, limits);
    agent.prepare_turn("Inspect once and report.").unwrap();
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
                        ("stdout", Value::string("x".repeat(80000))),
                        ("stderr", Value::string("")),
                    ])
                    .encode(),
                ),
            ),
        ]),
    ]);
    agent.context.observe(
        Some(&crate::backend::Usage {
            input_tokens: Some(6000),
            ..Default::default()
        }),
        2,
    );
    agent.context.calibrate(36000);
    let inbox = agent.inbox();
    let mut sent = false;
    agent
        .next_completion(Some(limits), &mut |event| {
            if matches!(&event, Event::ContextCompacted { text } if text.starts_with("Large tool results"))
            {
                send(&inbox, "Keep the original inspection scope.");
                sent = true;
            }
            Ok(())
        })
        .unwrap();
    let requests = fixture.finish();
    assert!(sent);
    assert_eq!(requests.len(), 1);
    assert!(requests[0].body.get("tools").is_some());
    let messages = requests[0]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        messages
            .last()
            .unwrap()
            .get("content")
            .and_then(Value::as_str),
        Some("Keep the original inspection scope.")
    );
    assert_eq!(agent.context.from, 1);
}

#[test]
fn input_during_a_summary_is_compacted_again_before_generation_when_needed() {
    let directory = Directory::new();
    let workspace = directory
        .path()
        .join("local-runner/jobs/jecode/fresh-checkout/project");
    std::fs::create_dir_all(&workspace).unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("Original objective retained.", vec![])),
        (
            200,
            completion("Original and updated objectives retained.", vec![]),
        ),
        (200, completion("Finished.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(&workspace).unwrap(),
    );
    let limits = Limits {
        context: agent.fixture_capacity(12_288, 80),
        output: Some(4096),
    };
    agent.client.fixture_limits(limits.context, limits.output);
    agent
        .prepare_turn("Continue the current inspection.")
        .unwrap();
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        (
            "content",
            Value::string("Recorded inspection result ".repeat(750)),
        ),
    ]));
    agent.context.observe(
        Some(&crate::backend::Usage {
            input_tokens: Some(22000),
            ..Default::default()
        }),
        2,
    );
    assert!(
        agent.context_status().tokens
            >= crate::context::threshold(limits.context, crate::context::AUTO_COMPACT_DEFAULT)
    );
    let inbox = agent.inbox();
    let update = "updated requirement ".repeat(900);
    let mut sent = false;
    let mut summaries_started = 0;
    agent
        .run_turn("Continue the current inspection.", &mut |event| {
            if matches!(event, Event::Recovering { .. }) {
                return Err("Unexpected fixture request".into());
            }
            if matches!(
                &event,
                Event::Maintenance { text } if text.starts_with("Compacting context")
            ) {
                summaries_started += 1;
                if !sent {
                    send(&inbox, &update);
                    sent = true;
                }
            }
            Ok(())
        })
        .unwrap();
    let requests = fixture.finish();
    assert!(sent);
    assert_eq!(summaries_started, 2);
    assert_eq!(requests.len(), 3);
    assert!(
        requests[..2]
            .iter()
            .all(|request| request.body.get("tools").is_none())
    );
    assert!(!requests[0].body.encode().contains("updated requirement"));
    assert!(requests[1].body.encode().contains("updated requirement"));
    assert!(requests[2].body.get("tools").is_some());
    assert!(requests[2].body.encode().contains("updated requirement"));
    assert_eq!(
        agent
            .messages
            .lock()
            .unwrap()
            .iter()
            .filter(
                |message| message.get("content").and_then(Value::as_str) == Some(update.as_str())
            )
            .count(),
        1
    );
}
