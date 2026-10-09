use super::*;
use crate::{
    config::{Settings, Store},
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, Response, completion},
    tools::Tools,
};

#[test]
#[ignore = "requires a real Windows console and manual composer/menu input"]
fn composer_windows_smoke() {
    let directory = Directory::new();
    eprintln!("Composer fixture: {}", directory.path().display());
    eprintln!(
        "Send a task, queue /help and a second task while busy. Then /model, search model-11, choose high; /effort low; /settings (Esc); /export; /exit."
    );
    let stream = Response::Stream(vec![
        (Duration::ZERO, ": OPENROUTER PROCESSING\n\n".into()),
        (Duration::from_secs(2), "data: {\"choices\":[{\"delta\":{\"reasoning\":\"isolated fixture reasoning\"},\"finish_reason\":null}]}\n\n".into()),
        (Duration::from_secs(4), "data: {\"choices\":[{\"delta\":{\"content\":\"A streamed fixture response.\"},\"finish_reason\":null}]}\n\n".into()),
        (Duration::from_secs(2), "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into()),
    ]);
    let fixture = HttpFixture::streaming_with_wait(
        vec![
            stream,
            Response::Json(
                200,
                completion(
                    &"A later fixture paragraph retained in the conversation.\n\n".repeat(30),
                    vec![],
                ),
            ),
            Response::Json(200, interaction_tests::catalog()),
            Response::Json(200, interaction_tests::catalog()),
        ],
        Duration::from_secs(180),
    );
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let store = Store::new(directory.path().join("config"));
    store.save(&settings).unwrap();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    run(
        agent,
        SessionConfig {
            store,
            settings,
            bash: crate::tools::find_bash().unwrap(),
        },
        Default::default(),
    )
    .unwrap();
    assert_eq!(fixture.finish().len(), 4);
    let export = std::fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("JECODE-SESSION-")
        })
        .expect("use /export");
    let exported = std::fs::read_to_string(export.path()).unwrap();
    assert!(!exported.contains("isolated-fixture-key"));
    assert!(exported.contains("fixture/model-11"));
    assert!(exported.contains("streamed fixture response"));
    assert!(exported.contains("\"effort\": \"low\""));
}

#[test]
#[ignore = "requires a real Windows console, native resizes and keyboard input"]
fn resize_windows_smoke() {
    use crate::{json::Value, test_support::tool_call};
    let directory = Directory::new();
    eprintln!("Resize fixture: {}", directory.path().display());
    eprintln!(
        "Send first task; queue /help and second task; resize while waiting, streaming and running the tool. Then /export, /new, /settings, Esc, /help, /exit."
    );
    let delta = |field: &str, text: &str| {
        format!(
            "data: {}\n\n",
            Value::object([(
                "choices",
                Value::Array(vec![Value::object([
                    ("delta", Value::object([(field, Value::string(text))])),
                    ("finish_reason", Value::Null),
                ])])
            )])
            .encode()
        )
    };
    let stream = Response::Stream(vec![
        (Duration::ZERO, ": fixture waiting\n\n".into()),
        (
            Duration::from_secs(3),
            delta("reasoning", "isolated reasoning"),
        ),
        (
            Duration::from_secs(3),
            delta(
                "content",
                "# Streamed fixture\nA response that wraps naturally when the terminal becomes narrower.\n\n```rust\nlet value = 1;\n",
            ),
        ),
        (
            Duration::from_secs(3),
            delta(
                "content",
                "println!(\"stream complete\");\n```\n\nFirst turn complete.",
            ),
        ),
        (
            Duration::from_secs(2),
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
                .into(),
        ),
    ]);
    let final_text = format!(
        "# Resize fixture completed\nThe command ran once.\n\n```rust\nlet value = 2;\n\nprintln!(\"verified\");\n```\n\n{}",
        (0..240)
            .map(|index| format!(
                "history-{index:03} A paragraph preserved across width and height changes.\n\n"
            ))
            .collect::<String>()
    );
    let fixture = HttpFixture::streaming_with_wait(
        vec![
            stream,
            Response::Json(
                200,
                completion(
                    "Running the fixture command.",
                    vec![tool_call(
                        "resize-command",
                        "bash",
                        Value::object([(
                            "command",
                            Value::string(
                                "printf 'executed\\n' >> executions.txt; printf 'resize tool output\\n'; sleep 4",
                            ),
                        )]),
                    )],
                ),
            ),
            Response::Json(200, completion(&final_text, vec![])),
        ],
        Duration::from_secs(180),
    );
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    run(agent, tests::config(&directory), Default::default()).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("executions.txt")).unwrap(),
        "executed\n"
    );
    let export = std::fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("JECODE-SESSION-")
        })
        .expect("export before /new");
    let exported = std::fs::read_to_string(export.path()).unwrap();
    assert!(!exported.contains("isolated-fixture-key"));
    assert!(exported.contains("First turn complete"));
    assert!(exported.contains("history-239"));
    assert!(exported.contains("resize tool output"));
    // /help is local and must not create provider requests while queued.
    assert!(!requests[1].body.encode().contains("Commands and controls"));
}
