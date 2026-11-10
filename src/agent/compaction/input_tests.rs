use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::inbox::Inbox;
use crate::test_support::{Directory, HttpFixture, completion};
use crate::tools::Tools;

fn send(inbox: &Inbox, text: &str) {
    inbox.with(|entries| {
        entries.push(text.into()).unwrap();
        entries.record();
    });
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
    let update = "updated requirement ".repeat(900);
    // The arriving request must fit after its second summary, while a recent
    // assistant tail still makes that second reduction necessary.
    let limits = Limits {
        context: agent.fixture_capacity(update.len() + 2048, 80),
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
            Value::string("Recorded inspection result ".repeat(900)),
        ),
    ]));
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        (
            "content",
            Value::string("Recent inspection detail ".repeat(200)),
        ),
    ]));
    agent.context.observe(
        Some(&crate::backend::Usage {
            input_tokens: Some(limits.context),
            ..Default::default()
        }),
        2,
    );
    assert!(
        agent.locked_estimate()
            >= crate::context::threshold(limits.context, crate::context::AUTO_COMPACT_DEFAULT)
    );
    let inbox = agent.inbox();
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
