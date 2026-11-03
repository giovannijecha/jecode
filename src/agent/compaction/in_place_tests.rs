use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::context::{AUTO_COMPACT_DEFAULT, threshold};
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use crate::tools::Tools;

const CAPACITY: usize = 60_000;

/// An agent whose conversation just crossed the auto-compact budget while
/// leaving room for an in-place summary.
fn crowded(fixture: &HttpFixture, directory: &Directory) -> Agent {
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.client.fixture_limits(CAPACITY, Some(4096));
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Start the long task.")),
    ]));
    let budget = threshold(CAPACITY, AUTO_COMPACT_DEFAULT);
    let mut index = 0;
    while agent.locked_estimate() < budget {
        agent.messages.lock().unwrap().push(Value::object([
            ("role", Value::string("assistant")),
            (
                "content",
                Value::string(format!("Step {index}: {}", "progress ".repeat(400))),
            ),
        ]));
        index += 1;
    }
    assert!(agent.locked_estimate() + 8192 < CAPACITY);
    agent
}

fn last_text(request: &crate::test_support::Request) -> String {
    request
        .body
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| messages.last())
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn proactive_compaction_summarizes_over_the_cached_conversation() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("Goal kept; next action: finish.", vec![])),
        (200, completion("Finished.", vec![])),
    ]);
    let mut agent = crowded(&fixture, &directory);
    agent.run_turn("Continue.", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    // The summary request is the conversation's own request plus one instruction,
    // with the same tool catalog, so its prefix is the cached one.
    let summary = &requests[0];
    assert!(summary.body.get("tools").is_some());
    assert!(last_text(summary).starts_with("Context checkpoint: Jecode is compacting"));
    assert!(summary.body.encode().contains("Step 0: progress"));
    assert_eq!(agent.context.summary, "Goal kept; next action: finish.");
    assert!(
        requests[1]
            .body
            .encode()
            .contains("Goal kept; next action: finish.")
    );
    assert!(!requests[1].body.encode().contains("Step 0: progress"));
}

#[test]
fn a_tool_call_instead_of_a_summary_falls_back_to_portions() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "w1",
                    "write",
                    Value::object([
                        ("path", Value::string("never.txt")),
                        ("content", Value::string("no")),
                    ]),
                )],
            ),
        ),
        // The crowded transcript needs two chronological portions.
        (200, completion("Goal kept from portions.", vec![])),
        (200, completion("Goal kept from portions.", vec![])),
        (200, completion("Finished.", vec![])),
    ]);
    let mut agent = crowded(&fixture, &directory);
    agent.run_turn("Continue.", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert!(requests[1].body.get("tools").is_none());
    assert!(
        requests[1]
            .body
            .encode()
            .contains("You are performing a context checkpoint compaction")
    );
    assert_eq!(agent.context.summary, "Goal kept from portions.");
    assert!(!directory.path().join("never.txt").exists());
}
