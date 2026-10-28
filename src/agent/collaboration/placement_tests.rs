use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    cancel::Cancellation,
    test_support::{Directory, HttpFixture, completion, tool_call},
};

type Stamps = (String, Option<String>, Option<bool>, Option<bool>);

/// A receipt's kind, the work label it records and whether a turn and a
/// call of the main agent ran.
fn stamps(receipt: &Value) -> Stamps {
    let flag = |key| match receipt.get(key) {
        Some(Value::Bool(flag)) => Some(*flag),
        _ => None,
    };
    (
        receipt
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        receipt.get("work").and_then(Value::as_str).map(Into::into),
        flag("in_turn"),
        flag("in_call"),
    )
}

#[test]
fn receipts_record_the_call_that_wrote_them_and_whether_a_turn_ran() {
    let project = Directory::new();
    let home = Directory::new();
    let review = Value::object([
        ("action", Value::string("review")),
        ("id", Value::string("a1")),
        ("report_version", Value::number(1)),
        ("note", Value::string("Checked the report")),
        ("work", Value::string("Review the search work")),
    ]);
    let fixture = HttpFixture::new(vec![
        (200, completion("Child report", vec![])),
        (
            200,
            completion("", vec![tool_call("review-1", "agent", review)]),
        ),
        (200, completion("Reviewed", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let mut create = super::tests::create();
    if let Value::Object(fields) = &mut create {
        fields.insert("work".into(), Value::string("Split the search work"));
    }
    parent.agent_tool(&create).unwrap();
    let observer = parent.collaboration();
    observer
        .wait(Some("a1"), Duration::from_secs(5), &Cancellation::default())
        .unwrap();
    parent
        .run_turn(
            crate::attachments::Prompt::plain("Review the report"),
            &mut |_| Ok(()),
        )
        .unwrap();
    parent
        .agent_tool(&Value::object([
            ("action", Value::string("close")),
            ("id", Value::string("a1")),
        ]))
        .unwrap();
    let expected: [Stamps; 5] = [
        ("created", Some("Split the search work"), false, false),
        // The child's own receipts belong to no call.
        ("received", None, false, false),
        ("returned", None, false, false),
        ("reviewed", Some("Review the search work"), true, true),
        // A call without a work label records an empty one.
        ("closed", Some(""), false, false),
    ]
    .map(|(kind, work, in_turn, in_call)| {
        (
            kind.into(),
            work.map(Into::into),
            Some(in_turn),
            Some(in_call),
        )
    });
    let receipts = observer.events_since(0).1;
    assert_eq!(receipts.iter().map(stamps).collect::<Vec<_>>(), expected);
    let saved = parent.sessions().unwrap().snapshot().events;
    let saved = saved
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("collaboration"))
        .map(stamps)
        .collect::<Vec<_>>();
    assert_eq!(saved, expected);
    assert_eq!(fixture.finish().len(), 3);
}
