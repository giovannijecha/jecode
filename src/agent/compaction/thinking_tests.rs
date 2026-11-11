//! A compaction boundary replaces the prefix that retained signed thinking was
//! bound to. Retained turns replay without it; later thinking stays valid.

use super::super::thinking_prefix_tests::{
    assert_append_only, assistant_content, bodies, catalog, claude_agent, images, read, reasoning,
    reply, screenshots, signed, text,
};
use super::super::*;
use crate::test_support::{Directory, HttpFixture};

#[test]
fn compaction_drops_retained_thinking_once_and_later_thinking_survives_resume() {
    let directory = Directory::new();
    let home = Directory::new();
    let shots = screenshots(&directory);
    let encoded = shots
        .each_ref()
        .map(|bytes| crate::attachments::base64::encode(bytes));
    let fixture = HttpFixture::streaming(vec![
        catalog(),
        reply(&[signed("", "sig-1"), read("toolu_a", r#"{"path":"a.png"}"#)]),
        reply(&[signed("", "sig-2"), text("A inspected.")]),
        reply(&[signed("", "sig-3"), read("toolu_b", r#"{"path":"b.png"}"#)]),
        reply(&[signed("", "sig-4"), text("B inspected.")]),
        reply(&[text("Screenshot A was inspected and matches.")]),
        reply(&[signed("", "sig-6"), read("toolu_c", r#"{"path":"c.png"}"#)]),
        reply(&[signed("", "sig-7"), text("C inspected.")]),
    ]);
    let mut agent = claude_agent(&fixture, &directory, &home);
    agent.run_turn("Inspect A", &mut |_| Ok(())).unwrap();
    agent.run_turn("Inspect B", &mut |_| Ok(())).unwrap();
    let messages = agent.messages.lock().unwrap().clone();
    let second = messages
        .iter()
        .position(|message| message.get("content").and_then(Value::as_str) == Some("Inspect B"))
        .unwrap();
    // The largest window whose recent tail starts at the second request.
    let limits = (1..=2_000)
        .rev()
        .map(|step| crate::backend::Limits {
            context: step * 5,
            output: None,
        })
        .find(|limits| {
            let budget = agent.context.budget(*limits, agent.auto_compact_percent()) / 4;
            agent.recent_boundary(&messages, budget) == second
        })
        .unwrap();
    assert!(
        agent
            .compact_if_needed(Some(limits), true, &mut |_| Ok(()))
            .unwrap()
    );
    assert_eq!(agent.context.from, second);
    agent.run_turn("Inspect C", &mut |_| Ok(())).unwrap();
    let id = agent.sessions().unwrap().id();
    let archived = Value::Array(agent.messages.lock().unwrap().clone()).encode();
    drop(agent);
    let requests = bodies(fixture);
    assert_eq!(requests.len(), 7);
    let (after, next) = (&requests[5], &requests[6]);
    // Retained turns keep text, tool calls and views, without bound thinking.
    assert!(reasoning(after).is_empty());
    let retained = Value::Array(assistant_content(after)).encode();
    assert!(retained.contains("toolu_b") && retained.contains("B inspected."));
    assert_eq!(images(after), encoded[1..2]);
    // Thinking produced after the boundary replays with an unchanged prefix.
    assert_append_only(after, next, "after compaction");
    assert_eq!(reasoning(next), ["sig-6"]);
    assert_eq!(images(next), encoded[1..]);
    // The archived transcript keeps every original block.
    for signature in ["sig-1", "sig-2", "sig-3", "sig-4", "sig-6", "sig-7"] {
        assert!(archived.contains(signature), "{signature}");
    }

    let resumed_fixture = HttpFixture::streaming(vec![
        catalog(),
        reply(&[signed("", "sig-8"), text("Resumed.")]),
    ]);
    let mut resumed = claude_agent(&resumed_fixture, &directory, &home);
    resumed.resume(&id).unwrap();
    resumed.run_turn("Check again", &mut |_| Ok(())).unwrap();
    let resumed_requests = bodies(resumed_fixture);
    assert_append_only(next, &resumed_requests[0], "resume after compaction");
    assert_eq!(reasoning(&resumed_requests[0]), ["sig-6", "sig-7"]);
    assert_eq!(images(&resumed_requests[0]), encoded[1..]);
}
