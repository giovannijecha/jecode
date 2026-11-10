//! Instruction snapshots through each native request mapping. A real agent
//! drives each backend against a loopback fixture; assertions read the bodies
//! actually sent. Prefix comparison normalizes only cache metadata:
//! `cache_control` breakpoints move to the newest blocks by design, and a
//! Messages API string content is shorthand for one text block, the form the
//! wire uses when a breakpoint sits on that message.

use super::*;
use crate::backend::{Backend, Service};
use crate::context::SNAPSHOT_HEADER;
use crate::json::Value;
use crate::test_support::{Directory, HttpFixture, Request, Response, completion, tool_call};
use crate::tools::Tools;
use std::time::Duration;

/// Native request parts: the instruction field and the ordered items.
struct Native<'a> {
    instructions: Option<&'a Value>,
    items: Vec<Value>,
}

fn strip_cache(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| key.as_str() != "cache_control")
                .map(|(key, value)| match (key.as_str(), value) {
                    ("content", Value::String(text)) if fields.contains_key("role") => (
                        key.clone(),
                        Value::Array(vec![Value::object([
                            ("text", Value::string(text)),
                            ("type", Value::string("text")),
                        ])]),
                    ),
                    _ => (key.clone(), strip_cache(value)),
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_cache).collect()),
        other => other.clone(),
    }
}

fn native<'a>(request: &'a Request, field: &str, items: &str) -> Native<'a> {
    let list = request.body.get(items).and_then(Value::as_array).unwrap();
    let skip = usize::from(field == "messages[0]");
    Native {
        instructions: if field == "messages[0]" {
            list.first()
        } else {
            request.body.get(field)
        },
        items: list[skip..].iter().map(strip_cache).collect(),
    }
}

fn is_snapshot(item: &Value) -> bool {
    item.encode().contains(SNAPSHOT_HEADER)
}

/// Runs the shared lifecycle: a tool exchange, then AGENTS.md creation, then
/// an unchanged turn. Returns the conversation requests, catalog reads excluded.
fn drive(client: Box<dyn Backend>, fixture: HttpFixture, directory: &Directory) -> Vec<Request> {
    fs::write(directory.path().join("note.txt"), "A note").unwrap();
    let mut agent = Agent::with_backend(client, Tools::new(directory.path()).unwrap());
    agent.run_turn("Read the note", &mut |_| Ok(())).unwrap();
    fs::write(directory.path().join(FILE_NAME), "Route rules v1").unwrap();
    agent.run_turn("Second", &mut |_| Ok(())).unwrap();
    agent.run_turn("Third", &mut |_| Ok(())).unwrap();
    fixture
        .finish()
        .into_iter()
        .filter(|request| request.headers[0].starts_with("POST "))
        .collect()
}

/// Shared assertions over the four conversation requests of `drive`.
fn assert_route(requests: &[Request], field: &str, items: &str, role: &str, replay: &[&str]) {
    assert_eq!(requests.len(), 4);
    let views = requests
        .iter()
        .map(|request| native(request, field, items))
        .collect::<Vec<_>>();
    for (index, pair) in views.windows(2).enumerate() {
        assert_eq!(
            pair[0].instructions, pair[1].instructions,
            "instructions {index}"
        );
        let (before, after) = (&pair[0].items, &pair[1].items);
        let diverged = before.iter().zip(after).position(|(a, b)| a != b);
        assert!(
            after.len() > before.len() && diverged.is_none(),
            "request {} does not extend request {index} at item {diverged:?}: {} vs {}",
            index + 1,
            diverged.map_or(String::new(), |at| before[at].encode()),
            diverged.map_or(String::new(), |at| after[at].encode()),
        );
    }
    assert!(
        !views[0]
            .instructions
            .unwrap()
            .encode()
            .contains("Route rules")
    );
    let created = &views[2].items;
    let snapshots = created.iter().filter(|item| is_snapshot(item)).count();
    assert_eq!(snapshots, 1);
    let at = created.iter().position(is_snapshot).unwrap();
    let prompt = created
        .iter()
        .position(|item| item.encode().contains("\"Second\""))
        .unwrap();
    assert!(prompt < at, "the snapshot follows the turn's prompt");
    assert!(created[at].encode().contains("Route rules v1"));
    assert_eq!(created[at].get("role").and_then(Value::as_str), Some(role));
    assert_eq!(
        views[3]
            .items
            .iter()
            .filter(|item| is_snapshot(item))
            .count(),
        1
    );
    // Tool pairing and model-native replay stay in the reused prefix.
    let replayed = Value::Array(created[..at].to_vec()).encode();
    for marker in replay {
        assert!(replayed.contains(marker), "missing {marker}");
    }
}

fn openai_stream(output: Vec<Value>) -> Response {
    let event = Value::object([
        ("type", Value::string("response.completed")),
        (
            "response",
            Value::object([
                ("status", Value::string("completed")),
                ("output", Value::Array(output)),
            ]),
        ),
    ]);
    Response::Stream(vec![(
        Duration::ZERO,
        format!("data: {}\n\n", event.encode()),
    )])
}

fn openai_message(text: &str) -> Value {
    Value::object([
        ("type", Value::string("message")),
        ("role", Value::string("assistant")),
        (
            "content",
            Value::Array(vec![Value::object([
                ("type", Value::string("output_text")),
                ("text", Value::string(text)),
            ])]),
        ),
    ])
}

fn openai_replies() -> Vec<Response> {
    vec![
        openai_stream(vec![
            Value::object([
                ("type", Value::string("reasoning")),
                ("encrypted_content", Value::string("opaque-reasoning")),
            ]),
            Value::object([
                ("type", Value::string("function_call")),
                ("namespace", Value::string("jecode")),
                ("call_id", Value::string("read-note")),
                ("name", Value::string("read")),
                ("arguments", Value::string(r#"{"path":"note.txt"}"#)),
            ]),
        ]),
        openai_stream(vec![openai_message("Read.")]),
        openai_stream(vec![openai_message("Second answer.")]),
        openai_stream(vec![openai_message("Third answer.")]),
    ]
}

const OPENAI_REPLAY: &[&str] = &["opaque-reasoning", "\"function_call_output\"", "read-note"];

#[test]
fn openai_account_sends_snapshots_as_developer_items_after_an_unchanged_prefix() {
    use crate::backend::openai::{Api, OpenAi};
    let directory = Directory::new();
    let fixture = HttpFixture::streaming(openai_replies());
    let api = Api::fixture(
        crate::auth::openai::Session::fixture("isolated-access-token"),
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    let client = Box::new(OpenAi::with_api(api, "gpt-6.1-sol".into()).unwrap());
    let requests = drive(client, fixture, &directory);
    assert_route(
        &requests,
        "instructions",
        "input",
        "developer",
        OPENAI_REPLAY,
    );
}

#[test]
fn openai_api_key_sends_snapshots_as_developer_items_after_an_unchanged_prefix() {
    use crate::backend::openai::{Api, OpenAi};
    let directory = Directory::new();
    let catalog = crate::json::parse(r#"{"data":[{"id":"gpt-6.1-sol"}]}"#).unwrap();
    let mut replies = vec![Response::Json(200, catalog)];
    replies.extend(openai_replies());
    let fixture = HttpFixture::streaming(replies);
    let api = Api::fixture_api_key(
        "isolated-fixture-api-key",
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    let client = Box::new(OpenAi::with_api(api, "gpt-6.1-sol".into()).unwrap());
    let requests = drive(client, fixture, &directory);
    assert_route(
        &requests,
        "instructions",
        "input",
        "developer",
        OPENAI_REPLAY,
    );
}

fn claude_event(value: Value) -> String {
    format!("data: {}\n\n", value.encode())
}

/// A content block with its `(delta type, field, value)` deltas.
type Block = (Value, Vec<(&'static str, &'static str, &'static str)>);

fn claude_stream(blocks: Vec<Block>, reason: &str) -> Response {
    let mut data = claude_event(
        crate::json::parse(
            r#"{"type":"message_start","message":{"usage":{"input_tokens":5,"output_tokens":1}}}"#,
        )
        .unwrap(),
    );
    for (index, (block, deltas)) in blocks.into_iter().enumerate() {
        data.push_str(&claude_event(Value::object([
            ("type", Value::string("content_block_start")),
            ("index", Value::number(index)),
            ("content_block", block),
        ])));
        for (kind, key, value) in deltas {
            data.push_str(&claude_event(Value::object([
                ("type", Value::string("content_block_delta")),
                ("index", Value::number(index)),
                (
                    "delta",
                    Value::object([("type", Value::string(kind)), (key, Value::string(value))]),
                ),
            ])));
        }
        data.push_str(&claude_event(Value::object([
            ("type", Value::string("content_block_stop")),
            ("index", Value::number(index)),
        ])));
    }
    data.push_str(&claude_event(Value::object([
        ("type", Value::string("message_delta")),
        (
            "delta",
            Value::object([("stop_reason", Value::string(reason))]),
        ),
        (
            "usage",
            Value::object([("output_tokens", Value::number(4))]),
        ),
    ])));
    data.push_str(&claude_event(Value::object([(
        "type",
        Value::string("message_stop"),
    )])));
    Response::Stream(vec![(Duration::ZERO, data)])
}

fn claude_text(text: &'static str) -> Response {
    let block = Value::object([("type", Value::string("text")), ("text", Value::string(""))]);
    claude_stream(
        vec![(block, vec![("text_delta", "text", text)])],
        "end_turn",
    )
}

fn claude_replies() -> Vec<Response> {
    let catalog = crate::json::parse(r#"{"data":[{"id":"claude-opus-5-5","max_input_tokens":1000000,"max_tokens":128000,"capabilities":{"effort":{"low":{"supported":true},"medium":{"supported":true},"high":{"supported":true}},"thinking":{"types":{"adaptive":{"supported":true},"disabled":{"supported":true}}}}}],"has_more":false}"#).unwrap();
    let thinking = Value::object([
        ("type", Value::string("thinking")),
        ("thinking", Value::string("")),
    ]);
    let tool = Value::object([
        ("type", Value::string("tool_use")),
        ("id", Value::string("toolu_note")),
        ("name", Value::string("read")),
        ("input", Value::object([])),
    ]);
    vec![
        Response::Json(200, catalog),
        claude_stream(
            vec![
                (
                    thinking,
                    vec![
                        ("thinking_delta", "thinking", "Read it"),
                        ("signature_delta", "signature", "signed-block"),
                    ],
                ),
                (
                    tool,
                    vec![("input_json_delta", "partial_json", r#"{"path":"note.txt"}"#)],
                ),
            ],
            "tool_use",
        ),
        claude_text("Read."),
        claude_text("Second answer."),
        claude_text("Third answer."),
    ]
}

const CLAUDE_REPLAY: &[&str] = &["signed-block", "\"tool_result\"", "toolu_note"];

#[test]
fn claude_account_sends_snapshots_as_midconversation_system_blocks() {
    let directory = Directory::new();
    let fixture = HttpFixture::streaming(claude_replies());
    let api = crate::backend::anthropic::Api::fixture(
        crate::auth::anthropic::Session::fixture("fixture-anthropic-token"),
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    let client = api.create("claude-opus-5-5".into()).unwrap();
    let requests = drive(client, fixture, &directory);
    assert_route(&requests, "system", "messages", "system", CLAUDE_REPLAY);
}

#[test]
fn claude_api_key_sends_snapshots_as_user_turns() {
    let directory = Directory::new();
    let fixture = HttpFixture::streaming(claude_replies());
    let api = crate::backend::anthropic::Api::fixture_api_key(
        "isolated-fixture-api-key",
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    let client = api.create("claude-opus-5-5".into()).unwrap();
    let requests = drive(client, fixture, &directory);
    assert_route(&requests, "system", "messages", "user", CLAUDE_REPLAY);
}

#[test]
fn openrouter_sends_snapshots_as_user_messages() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "read-note",
                    "read",
                    Value::object([("path", Value::string("note.txt"))]),
                )],
            ),
        ),
        (200, completion("Read.", vec![])),
        (200, completion("Second answer.", vec![])),
        (200, completion("Third answer.", vec![])),
    ]);
    let client = Box::new(crate::backend::openrouter::OpenRouter::fixture(
        fixture.endpoint.clone(),
    ));
    let requests = drive(client, fixture, &directory);
    assert_route(
        &requests,
        "messages[0]",
        "messages",
        "user",
        &["read-note", "\"tool\""],
    );
}
