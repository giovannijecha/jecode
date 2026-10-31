//! Explicit subscription checks use only an isolated, selected Jecode account.

use super::*;

const MODEL: &str = "claude-opus-5-5";
const MARKER: &str = "JECODE_ACCOUNT_PROBE_OK";

fn client() -> Box<dyn Backend> {
    let home = std::env::var_os("JECODE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|home| home.is_absolute())
        .expect("Set JECODE_HOME to an isolated signed-in Jecode home");
    let session = crate::auth::anthropic::Store::new(home)
        .active()
        .unwrap()
        .expect("Complete Claude sign-in in the isolated home first");
    let api = Api::new(session).unwrap();
    let usage = api.get("/api/oauth/usage").unwrap();
    assert_eq!(
        usage
            .get("extra_usage")
            .and_then(|value| value.get("is_enabled")),
        Some(&Value::Bool(false)),
        "This bounded live check requires extra usage disabled"
    );
    api.catalog().unwrap();
    api.create(MODEL.into()).unwrap()
}

#[test]
#[ignore = "uses the explicitly selected Claude account for one synthetic request"]
fn minimal_subscription_stream() {
    let backend = client();
    let result = backend
        .sample(
            &[Value::object([
                ("role", Value::string("user")),
                (
                    "content",
                    Value::string(format!("Reply with exactly {MARKER} and nothing else.")),
                ),
            ])],
            false,
            Some(64),
            None,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(result.text.trim(), MARKER);
    assert!(result.calls.is_empty());
    println!(
        "Claude minimal stream completed; output tokens {:?}",
        result.usage.and_then(|usage| usage.output_tokens)
    );
}

#[test]
#[ignore = "uses the explicitly selected Claude account for two synthetic tool requests"]
fn subscription_tool_roundtrip() {
    let backend = client();
    let user = Value::object([
        ("role", Value::string("user")),
        (
            "content",
            Value::string(
                "Use only the read tool once to read PROBE.txt. After receiving its result, reply with exactly its contents. Do not use other tools.",
            ),
        ),
    ]);
    let first = backend
        .sample(
            std::slice::from_ref(&user),
            true,
            Some(1024),
            None,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(first.calls.len(), 1);
    assert_eq!(first.calls[0].name, "read");
    let receipt = Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string(&first.calls[0].id)),
        ("content", Value::string(MARKER)),
    ]);
    let second = backend
        .sample(
            &[user, first.message, receipt],
            true,
            Some(1024),
            None,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(second.text.trim(), MARKER);
    assert!(second.calls.is_empty());
    println!("Claude read tool and signed native replay completed");
}
