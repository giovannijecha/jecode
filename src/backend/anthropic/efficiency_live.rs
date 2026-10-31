//! Bounded account checks use synthetic text and print counters only.
use super::*;
use std::time::Instant;

fn remove_cache(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            fields.remove("cache_control");
            for value in fields.values_mut() {
                remove_cache(value);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(remove_cache),
        _ => {}
    }
}

fn run(
    api: &Api,
    messages: &[Value],
    cache: bool,
    previous: Option<&str>,
) -> (backend::Completion, Option<String>) {
    let mut request = wire::request(
        "claude-opus-5-5",
        Effort::Medium,
        messages,
        None,
        Some(64),
        None,
        None,
    )
    .unwrap();
    if !cache {
        remove_cache(&mut request);
    }
    if let Value::Object(fields) = &mut request {
        fields.insert(
            "diagnostics".into(),
            Value::object([(
                "previous_message_id",
                previous.map_or(Value::Null, Value::string),
            )]),
        );
    }
    let mut stream = stream::Stream::default();
    let start = Instant::now();
    let mut first = None;
    let mut events = Vec::new();
    api.post_stream(&request.encode(), &mut |bytes| {
        events.extend_from_slice(bytes);
        stream.feed(bytes, &mut |update| {
            if matches!(
                update,
                backend::Update::TextDelta(_)
                    | backend::Update::Reasoning
                    | backend::Update::Working
            ) {
                first.get_or_insert_with(|| start.elapsed());
            }
            Ok(())
        })
    })
    .unwrap();
    let result = stream.finish().unwrap();
    let message_start = String::from_utf8(events)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|line| crate::json::parse(line).ok())
        .find(|event| event.get("type").and_then(Value::as_str) == Some("message_start"));
    let id = message_start
        .as_ref()
        .and_then(|event| event.get("message")?.get("id")?.as_str())
        .map(str::to_owned);
    let diagnostics = message_start
        .as_ref()
        .and_then(|event| event.get("message")?.get("diagnostics"))
        .map(Value::encode);
    assert_eq!(result.text.trim(), "READY");
    println!(
        "cache={cache} wall_ms={} first_ms={:?} usage={:?} diagnostics={diagnostics:?}",
        start.elapsed().as_millis(),
        first.map(|v| v.as_millis()),
        result.usage
    );
    (result, id)
}

#[test]
#[ignore = "uses four bounded synthetic Claude requests on the explicitly selected account"]
fn durable_prefix_cache_survives_runtime_state_changes() {
    let home = std::env::var_os("JECODE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .expect("Set JECODE_HOME explicitly");
    let session = crate::auth::anthropic::Store::new(home)
        .active()
        .unwrap()
        .expect("Sign in to Claude first");
    let api = Api::new(session).unwrap();
    let usage = api.get("/api/oauth/usage").unwrap();
    assert_eq!(
        usage
            .get("extra_usage")
            .and_then(|value| value.get("is_enabled")),
        Some(&Value::Bool(false))
    );
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut ledger = format!(
        "Synthetic cache check {nonce}. The following ledger is reference data. Reply with exactly READY.\n"
    );
    for i in 0..900 {
        ledger.push_str(&format!("Record {i:04}: the stable identifier is item-{i:04}; preserve its recorded value and original evidence reference.\n"));
    }
    let mut messages = vec![
        Value::object([
            ("role", Value::string("system")),
            (
                "content",
                Value::string(
                    "Read the synthetic reference data and reply with exactly READY. No tools.",
                ),
            ),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string(ledger)),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("jecode_request_state", Value::Bool(true)),
            (
                "content",
                Value::string("Jecode session state for the next request. Revision 1."),
            ),
        ]),
    ];
    let (baseline, _) = run(&api, &messages, false, None);
    assert_eq!(baseline.usage.unwrap().cached_tokens.unwrap_or(0), 0);
    let (cold, cold_id) = run(&api, &messages, true, None);
    assert!(cold.usage.unwrap().cache_write_tokens.unwrap_or(0) > 0);
    let (repeated, repeated_id) = run(&api, &messages, true, cold_id.as_deref());
    let mut state = messages.pop().unwrap();
    messages.push(cold.message);
    messages.push(Value::object([
        ("role", Value::string("user")),
        (
            "content",
            Value::string("Continue the same ledger check. Reply with exactly READY."),
        ),
    ]));
    if let Value::Object(fields) = &mut state {
        fields.insert(
            "content".into(),
            Value::string("Jecode session state for the next request. Revision 2."),
        );
    }
    messages.push(state);
    let (warm, _) = run(&api, &messages, true, repeated_id.as_deref());
    assert!(repeated.usage.unwrap().cached_tokens.unwrap_or(0) > 0);
    assert!(warm.usage.unwrap().cached_tokens.unwrap_or(0) > 0);
}
