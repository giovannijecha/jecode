//! Esc through the real ConPTY loop while a streamed turn updates long history.
use super::*;
use crate::{json::Value, test_support::Response};
use std::{path::Path, sync::mpsc};

#[test]
#[ignore = "requires Windows ConPTY for active long-history interruption"]
fn native_windows_escape_during_active_long_history() {
    let directory = Directory::new();
    let executable = std::env::current_exe().unwrap();
    let result = native_script::pty(
        native_pty_source(),
        "Run",
        &[
            &executable.to_string_lossy(),
            &directory.path().to_string_lossy(),
            "active-long-history",
        ],
        &directory,
        Duration::from_secs(60),
        false,
    );
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(!result.timed_out);
}

pub(super) fn console(path: &Path) {
    let home = path.join("home");
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let config = SessionConfig {
        store: Store::new(home.clone()),
        settings,
    };
    config.store.save(&config.settings).unwrap();
    std::fs::write(path.join("evidence.txt"), "NATIVE_TOOL_OUTPUT_MARKER\n").unwrap();

    // Keep the second response open after its first visible text. This makes
    // Esc target an active worker rather than a completed turn.
    let (_release, held) = mpsc::channel::<()>();
    let answer = format!(
        "NATIVE_STREAM_HEAD\n{}\nNATIVE_STREAM_HEAD",
        (0..1200)
            .map(|line| format!("- streamed item {line:04} has **formatted** content.\n"))
            .collect::<String>()
    );
    let head = format!(
        "data: {}\n\n",
        Value::object([(
            "choices",
            Value::Array(vec![Value::object([
                ("delta", Value::object([("content", Value::string(answer))])),
                ("finish_reason", Value::Null),
            ])]),
        )])
        .encode()
    );
    let fixture = HttpFixture::streaming_with_wait(
        vec![
            Response::Json(
                200,
                completion(
                    "",
                    vec![tool_call(
                        "native-history-read",
                        "read",
                        Value::object([("path", Value::string("evidence.txt"))]),
                    )],
                ),
            ),
            Response::GatedStream {
                head,
                tail: "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),
                release: held,
            },
        ],
        Duration::from_secs(30),
    );
    let agent = || {
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(path).unwrap(),
        )
    };
    let mut saved = agent();
    saved.enable_sessions(&home).unwrap();
    saved
        .archive()
        .messages
        .lock()
        .unwrap()
        .extend(crate::tui::measurement_tests::mixed_messages(4));
    saved.save_session().unwrap();
    let id = saved.sessions().unwrap().snapshot().id.clone();
    drop(saved);

    crate::tui::launch(
        agent(),
        config,
        Some(Some(id.clone())),
        Default::default(),
        None,
    )
    .unwrap();
    let store = crate::sessions::Store::new(home, path).unwrap();
    let session = store.fixture_load(&id).unwrap();
    assert_eq!(session.input.draft.text, "kept draft");
    assert!(session.messages.iter().any(|message| {
        message.get("tool_call_id").and_then(Value::as_str) == Some("native-history-read")
            && message
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("NATIVE_TOOL_OUTPUT_MARKER"))
    }));
    assert_eq!(fixture.finish().len(), 2);
}
