//! Bounded SIWC protocol/cache checks use synthetic text and counters only.
use super::*;
use std::time::Instant;

fn run(
    api: &Api,
    conversation: &str,
    messages: &[Value],
    separate_state: bool,
) -> backend::Completion {
    let mut request =
        wire::request_with_catalog("gpt-6.1-sol", Effort::Low, messages, None, None).unwrap();
    if !separate_state
        && let Value::Object(fields) = &mut request
        && let Some(Value::Array(input)) = fields.get_mut("input")
        && let Some(Value::Object(last)) = input.last_mut()
    {
        last.insert("role".into(), Value::string("user"));
    }
    if let Value::Object(fields) = &mut request {
        fields.insert("prompt_cache_key".into(), Value::string(conversation));
    }
    let mut stream = stream::Stream::default();
    let start = Instant::now();
    let mut first = None;
    api.post_stream(
        "/responses",
        &request.encode(),
        Some(conversation),
        &mut |bytes| {
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
        },
    )
    .unwrap();
    let result = stream.finish().unwrap();
    assert_eq!(result.text.trim(), "READY");
    println!(
        "separate_state={separate_state} wall_ms={} first_ms={:?} usage={:?}",
        start.elapsed().as_millis(),
        first.map(|v| v.as_millis()),
        result.usage
    );
    result
}

#[test]
#[ignore = "uses three small synthetic SIWC requests on the explicitly selected account"]
fn durable_prefix_cache_survives_runtime_state_changes() {
    let home = std::env::var_os("JECODE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .expect("Set JECODE_HOME explicitly");
    let session = crate::auth::openai::Store::new(home)
        .active()
        .unwrap()
        .expect("Sign in to ChatGPT first");
    let api = Api::new(session).unwrap();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let conversation = format!("jecode-cache-check-{nonce}");
    let mut ledger = format!("Synthetic cache check {nonce}. Reply with exactly READY.\n");
    for i in 0..600 {
        ledger.push_str(&format!("Record {i:04}: the stable identifier is item-{i:04}; preserve the recorded value and evidence reference.\n"));
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
    run(&api, &conversation, &messages, false);
    let cold = run(&api, &conversation, &messages, true);
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
    let warm = run(&api, &conversation, &messages, true);
    assert!(warm.usage.unwrap().cached_tokens.unwrap_or(0) > 0);
}
