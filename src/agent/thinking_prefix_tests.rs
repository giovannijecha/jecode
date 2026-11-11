//! Anthropic signs each thinking block over the exact prefix sent before it.
//! Later requests must replay that prefix: earlier tool views, native response
//! blocks and their order. Only documented equivalences are normalized here.

use super::*;
use crate::backend::Service;
use crate::sessions::Store;
use crate::test_support::{Directory, HttpFixture, Response};
use std::time::Duration;

const KEY: &str = "fixture-anthropic-api-key";
const MODEL: &str = "claude-fable-5-2";

mod subscription_tests;

pub(super) fn catalog() -> Response {
    Response::Json(
        200,
        crate::json::parse(
            r#"{"data":[{"id":"claude-fable-5-2","display_name":"Claude Fable 5.2","max_input_tokens":1000000,"max_tokens":128000,"capabilities":{"effort":{"low":{"supported":true},"medium":{"supported":true},"high":{"supported":true}},"thinking":{"types":{"adaptive":{"supported":true}}}}}],"has_more":false}"#,
        )
        .unwrap(),
    )
}

pub(super) fn signed(text: &str, signature: &str) -> Value {
    Value::object([
        ("type", Value::string("thinking")),
        ("thinking", Value::string(text)),
        ("signature", Value::string(signature)),
    ])
}

pub(super) fn redacted(data: &str) -> Value {
    Value::object([
        ("type", Value::string("redacted_thinking")),
        ("data", Value::string(data)),
    ])
}

pub(super) fn text(text: &str) -> Value {
    Value::object([
        ("type", Value::string("text")),
        ("text", Value::string(text)),
    ])
}

pub(super) fn read(id: &str, input: &str) -> Value {
    Value::object([
        ("type", Value::string("tool_use")),
        ("id", Value::string(id)),
        ("name", Value::string("read")),
        ("input", crate::json::parse(input).unwrap()),
    ])
}

/// A streamed Messages response that carries `blocks` exactly.
pub(super) fn reply(blocks: &[Value]) -> Response {
    let event = |value: Value| format!("data: {}\n\n", value.encode());
    let calls = blocks
        .iter()
        .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"));
    let mut data = event(crate::json::parse(
        r#"{"type":"message_start","message":{"usage":{"input_tokens":100,"output_tokens":1}}}"#,
    ).unwrap());
    for (index, block) in blocks.iter().enumerate() {
        data.push_str(&event(Value::object([
            ("type", Value::string("content_block_start")),
            ("index", Value::number(index)),
            ("content_block", block.clone()),
        ])));
        data.push_str(&event(Value::object([
            ("type", Value::string("content_block_stop")),
            ("index", Value::number(index)),
        ])));
    }
    let stop = if calls { "tool_use" } else { "end_turn" };
    data.push_str(&event(
        crate::json::parse(&format!(
            r#"{{"type":"message_delta","delta":{{"stop_reason":"{stop}"}},"usage":{{"output_tokens":9}}}}"#
        ))
        .unwrap(),
    ));
    data.push_str(&event(
        crate::json::parse(r#"{"type":"message_stop"}"#).unwrap(),
    ));
    Response::Stream(vec![(Duration::ZERO, data)])
}

pub(super) fn claude_agent(
    fixture: &HttpFixture,
    directory: &Directory,
    home: &Directory,
) -> Agent {
    let api = crate::backend::anthropic::Api::fixture_api_key(
        KEY,
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    assert_eq!(Service::id(&api), "anthropic-api");
    let mut agent = Agent::with_backend(
        api.create(MODEL.into()).unwrap(),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
}

/// Message request bodies in the order sent, without catalog reads.
pub(super) fn bodies(fixture: HttpFixture) -> Vec<Value> {
    fixture
        .finish()
        .into_iter()
        .filter(|request| request.headers[0].starts_with("POST /v1/messages"))
        .map(|request| request.body)
        .collect()
}

/// Cache markers are not part of the binding.
fn unmarked(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| key.as_str() != "cache_control")
                .map(|(key, value)| (key.clone(), unmarked(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(unmarked).collect()),
        other => other.clone(),
    }
}

/// String content and a single text block of the same text are equivalent.
fn shape(message: &Value) -> Value {
    let mut message = unmarked(message);
    if let Value::Object(fields) = &mut message
        && let Some(Value::String(content)) = fields.get("content").cloned()
    {
        fields.insert("content".into(), Value::Array(vec![text(&content)]));
    }
    message
}

pub(super) fn messages(body: &Value) -> &[Value] {
    body.get("messages").and_then(Value::as_array).unwrap()
}

fn summary(message: &Value) -> String {
    let blocks = shape(message);
    let types = blocks
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| block.get("type").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    format!(
        "{}[{types}]",
        message.get("role").and_then(Value::as_str).unwrap_or("?")
    )
}

/// The later request repeats everything the earlier one sent, then appends.
pub(super) fn assert_append_only(earlier: &Value, later: &Value, label: &str) {
    assert!(
        earlier.get("system").map(unmarked) == later.get("system").map(unmarked),
        "{label}: system changed"
    );
    assert!(
        earlier.get("tools") == later.get("tools"),
        "{label}: tools changed"
    );
    let (before, after) = (messages(earlier), messages(later));
    assert!(
        after.len() > before.len(),
        "{label}: {} messages became {}",
        before.len(),
        after.len()
    );
    for (index, (sent, replayed)) in before.iter().zip(after).enumerate() {
        assert!(
            shape(sent) == shape(replayed),
            "{label}: messages[{index}] {} became {}",
            summary(sent),
            summary(replayed)
        );
    }
}

fn blocks_of<'a>(body: &'a Value, kind: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
    messages(body)
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_array))
        .flatten()
        .filter(move |block| block.get("type").and_then(Value::as_str) == Some(kind))
}

/// Base64 image data in request order.
pub(super) fn images(body: &Value) -> Vec<String> {
    blocks_of(body, "image")
        .filter_map(|block| block.get("source")?.get("data")?.as_str())
        .map(str::to_owned)
        .collect()
}

/// Thinking signatures and redacted payloads in request order.
pub(super) fn reasoning(body: &Value) -> Vec<String> {
    blocks_of(body, "thinking")
        .chain(blocks_of(body, "redacted_thinking"))
        .filter_map(|block| {
            block
                .get("signature")
                .or_else(|| block.get("data"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
        .collect()
}

pub(super) fn assistant_content(body: &Value) -> Vec<Value> {
    messages(body)
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .map(|message| unmarked(message.get("content").unwrap()))
        .collect()
}

pub(super) fn screenshots(directory: &Directory) -> [Vec<u8>; 3] {
    let shots = [
        crate::attachments::tests::png(4, 3),
        crate::attachments::tests::png(5, 4),
        crate::attachments::tests::png(6, 5),
    ];
    for (name, bytes) in ["a.png", "b.png", "c.png"].iter().zip(&shots) {
        std::fs::write(directory.path().join(name), bytes).unwrap();
    }
    std::fs::write(directory.path().join("notes.txt"), "fixture notes\n").unwrap();
    shots
}

#[test]
fn screenshots_and_native_blocks_stay_in_the_signed_prefix_across_turns_and_resume() {
    let directory = Directory::new();
    let home = Directory::new();
    let shots = screenshots(&directory);
    let encoded = shots
        .each_ref()
        .map(|bytes| crate::attachments::base64::encode(bytes));
    // JSON text in an array field is decoded for execution but must replay as sent.
    let first = [
        signed("Inspect the first screenshot", "sig-1"),
        redacted("redacted-1"),
        text("Reading the first screenshot and notes."),
        read("toolu_a", r#"{"path":"a.png"}"#),
        read("toolu_notes", r#"{"paths":"[\"notes.txt\"]"}"#),
    ];
    let second = [
        signed("Compare with the second screenshot", "sig-2"),
        read("toolu_b", r#"{"path":"b.png"}"#),
        read("toolu_again", r#"{"paths":"[\"notes.txt\"]"}"#),
    ];
    let third = [signed("", "sig-3"), read("toolu_c", r#"{"path":"c.png"}"#)];
    let fourth = [signed("", "sig-4"), text("All three screenshots match.")];
    let fixture = HttpFixture::streaming(vec![
        catalog(),
        reply(&first),
        reply(&second),
        reply(&third),
        reply(&fourth),
    ]);
    let mut agent = claude_agent(&fixture, &directory, &home);
    agent
        .run_turn("Compare the screenshots", &mut |_| Ok(()))
        .unwrap();
    let id = agent.sessions().unwrap().id();
    let stored = agent.messages.lock().unwrap().clone();
    // The context estimate charges every screenshot the next request repeats.
    let inputs = agent.client.inputs();
    let views = stored
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .map(|message| crate::attachments::weight_for(message, inputs))
        .filter(|weight| *weight > 0)
        .count();
    let text_bytes = stored
        .iter()
        .map(|message| crate::context::portable(message).encode().len())
        .sum::<usize>();
    assert_eq!(views, 3);
    assert_eq!(
        crate::context::bytes_for(&stored, inputs),
        text_bytes
            + stored
                .iter()
                .map(|message| crate::attachments::weight_for(message, inputs))
                .sum::<usize>()
    );
    drop(agent);
    let requests = bodies(fixture);
    assert_eq!(requests.len(), 4);
    for (index, pair) in requests.windows(2).enumerate() {
        assert_append_only(
            &pair[0],
            &pair[1],
            &format!("R{}->R{}", index + 1, index + 2),
        );
    }
    // Each screenshot is sent once it is read and stays in every later request.
    assert_eq!(images(&requests[1]), encoded[..1]);
    assert_eq!(images(&requests[2]), encoded[..2]);
    assert_eq!(images(&requests[3]), encoded);
    // Native blocks replay exactly as received, including redacted thinking.
    assert_eq!(
        assistant_content(&requests[3]),
        [
            Value::Array(first.to_vec()),
            Value::Array(second.to_vec()),
            Value::Array(third.to_vec())
        ]
    );
    assert_eq!(
        reasoning(&requests[3]),
        ["sig-1", "sig-2", "sig-3", "redacted-1"]
    );
    // The decoded copy executed: the batched read returned the notes.
    let notes = stored
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some("toolu_notes"))
        .and_then(|message| message.get("content").and_then(Value::as_str))
        .unwrap();
    assert!(notes.contains("fixture notes"), "{notes}");

    // Save and resume: the same content, attachment bytes and order.
    let resumed_fixture = HttpFixture::streaming(vec![
        catalog(),
        reply(&[signed("", "sig-5"), text("Still consistent.")]),
    ]);
    let mut resumed = claude_agent(&resumed_fixture, &directory, &home);
    resumed.resume(&id).unwrap();
    resumed.run_turn("Check again", &mut |_| Ok(())).unwrap();
    let after = bodies(resumed_fixture);
    assert_append_only(&requests[3], &after[0], "resume");
    assert_eq!(images(&after[0]), encoded);
    assert_eq!(
        reasoning(&after[0]),
        ["sig-1", "sig-2", "sig-3", "sig-4", "redacted-1"]
    );
    assert!(
        Store::new(home.path().to_path_buf(), directory.path())
            .unwrap()
            .fixture_load(&id)
            .is_ok()
    );
}

#[test]
fn continuation_and_followup_tool_results_preserve_the_signed_prefix() {
    let directory = Directory::new();
    let home = Directory::new();
    screenshots(&directory);
    let fixture = HttpFixture::streaming(vec![
        catalog(),
        reply(&[signed("", "sig-1"), text("Initial inspection complete.")]),
        reply(&[signed("", "sig-2"), read("toolu_a", r#"{"path":"a.png"}"#)]),
        reply(&[signed("", "sig-3"), text("Follow-up complete.")]),
        reply(&[signed("", "sig-4"), read("toolu_b", r#"{"path":"b.png"}"#)]),
        reply(&[signed("", "sig-5"), text("Second follow-up complete.")]),
    ]);
    let mut agent = claude_agent(&fixture, &directory, &home);
    agent.run_turn("Inspect the page", &mut |_| Ok(())).unwrap();
    agent.continue_turn(&mut |_| Ok(())).unwrap();
    agent.continue_turn(&mut |_| Ok(())).unwrap();
    assert_eq!(
        agent
            .context
            .states
            .iter()
            .filter(|state| state.text == crate::context::CONTINUATION)
            .count(),
        2
    );
    assert_ne!(
        agent.context.latest_state(false).unwrap().text,
        crate::context::CONTINUATION
    );
    let id = agent.sessions().unwrap().id();
    drop(agent);
    let requests = bodies(fixture);
    assert_eq!(requests.len(), 5);
    for pair in requests.windows(2) {
        assert_append_only(&pair[0], &pair[1], "continuation");
    }

    let resumed_fixture = HttpFixture::streaming(vec![
        catalog(),
        reply(&[signed("", "sig-6"), read("toolu_c", r#"{"path":"c.png"}"#)]),
        reply(&[signed("", "sig-7"), text("Resumed follow-up complete.")]),
    ]);
    let mut resumed = claude_agent(&resumed_fixture, &directory, &home);
    resumed.resume(&id).unwrap();
    resumed.continue_turn(&mut |_| Ok(())).unwrap();
    let after = bodies(resumed_fixture);
    assert_eq!(after.len(), 2);
    assert_append_only(requests.last().unwrap(), &after[0], "resume continuation");
    assert_append_only(&after[0], &after[1], "resumed tool result");
}
