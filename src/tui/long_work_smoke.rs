use super::*;
use crate::{
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, Response, completion, tool_call},
    tools::Tools,
};

#[test]
#[ignore = "requires an isolated real Windows console and native keyboard input"]
fn long_work_windows_smoke() {
    let directory = Directory::new();
    let evidence = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/long-work-smoke");
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::write(
        evidence.join("FIXTURE.txt"),
        directory.path().to_string_lossy().as_bytes(),
    )
    .unwrap();
    let mut call = completion(
        "Running the foreground command.",
        vec![tool_call(
            "once",
            "bash",
            Value::object([(
                "command",
                Value::string("printf 'once\\n' >> executions.txt; printf '%70000s' x; sleep 2"),
            )]),
        )],
    );
    if let Value::Object(fields) = &mut call {
        fields.insert(
            "usage".into(),
            Value::object([
                ("prompt_tokens", Value::number(5000)),
                ("completion_tokens", Value::number(100)),
            ]),
        );
    }
    let mut responses = vec![
        Response::Http(
            503,
            vec![("Retry-After".into(), "3".into())],
            Value::object([(
                "error",
                Value::object([("message", Value::string("Temporary fixture outage"))]),
            )]),
        ),
        Response::Json(200, call),
    ];
    responses.push(Response::Json(
        200,
        completion(
            &String::from("The foreground command ran once; report its result."),
            vec![],
        ),
    ));
    responses.push(Response::Json(200, completion("# Long work fixture completed\nThe command ran once, the connection recovered and original output is retained.\n\n```rust\nlet verified = true;\n```", vec![])));
    let fixture = HttpFixture::streaming_with_wait(responses, Duration::from_secs(180));
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_limits(24000, Some(4096));
    run(
        Agent::new(client, Tools::new(directory.path()).unwrap()),
        tests::config(&directory),
        Default::default(),
    )
    .unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("executions.txt")).unwrap(),
        "once\n"
    );
    let home = directory.path().join(".jecode");
    let store = crate::sessions::Store::new(home, directory.path()).unwrap();
    let id = store.list().unwrap().sessions[0].id.clone();
    let saved = store.fixture_load(&id).unwrap();
    assert!(saved.context.from > 1);
    assert_eq!(saved.input.draft.text, "draft kept");
    assert!(saved.input.queued.is_empty());
    assert!(
        saved
            .events
            .iter()
            .any(|event| event.get("command").and_then(Value::as_str) == Some("/help"))
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
        .unwrap();
    let text = std::fs::read_to_string(export.path()).unwrap();
    assert!(text.contains("request_recovery"));
    assert!(text.contains("Context compaction"));
    assert!(text.contains("stdout_file"));
    assert!(!text.contains("isolated-fixture-key"));
    std::fs::write(evidence.join("export.json"), text).unwrap();
    std::fs::write(
        evidence.join("PASSED.txt"),
        "Native retry, compaction, queued local command, draft, resize and export checks passed.",
    )
    .unwrap();
}
