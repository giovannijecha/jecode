use super::*;
use crate::{
    effort::Effort,
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, Response, completion, tool_call},
    tools::Tools,
};

#[test]
#[ignore = "requires a real Windows console; close and resume with manual input"]
fn sessions_windows_smoke() {
    let directory = Directory::new();
    eprintln!("Sessions fixture: {}", directory.path().display());
    eprintln!(
        "Send first task, wait for the answer. Send second task; during its stream queue /help and queued follow-up, then type draft kept and Ctrl+Q. Choose the saved session, inspect recovered input, clear it, /export and Ctrl+Q."
    );
    let partial = format!(
        "data: {}\n\n",
        Value::object([(
            "choices",
            Value::Array(vec![Value::object([
                (
                    "delta",
                    Value::object([(
                        "content",
                        Value::string("A partial second answer, preserved for recovery.")
                    )])
                ),
                ("finish_reason", Value::Null),
            ])])
        )])
        .encode()
    );
    let fixture = HttpFixture::streaming_with_wait(
        vec![
            Response::Json(
                200,
                completion(
                    "Running the fixture command.",
                    vec![tool_call(
                        "once",
                        "bash",
                        Value::object([(
                            "command",
                            Value::string(
                                "printf 'once\\n' >> executions.txt; printf 'saved tool output\\n'",
                            ),
                        )]),
                    )],
                ),
            ),
            Response::Json(
                200,
                completion(
                    "# Saved answer\nThe command ran once.\n\n```rust\nlet value = 1;\n```\n\nReady for the next task.",
                    vec![],
                ),
            ),
            Response::Stream(vec![
                (Duration::from_secs(1), partial),
                (
                    Duration::from_secs(45),
                    ": fixture still waiting\n\n".into(),
                ),
                (Duration::from_secs(1), "data: [DONE]\n\n".into()),
            ]),
        ],
        Duration::from_secs(180),
    );
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.set_effort(Effort::High);
    run(
        Agent::new(client, Tools::new(directory.path()).unwrap()),
        tests::config(&directory),
    )
    .unwrap();
    let home = directory.path().join(".jecode");
    let bucket = std::fs::read_dir(home.join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = std::fs::read_dir(bucket)
        .unwrap()
        .flatten()
        .find(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .unwrap()
        .path();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("isolated-fixture-key"));
    let document = crate::sessions::Store::new(home, directory.path())
        .unwrap()
        .fixture_load(path.file_stem().unwrap().to_str().unwrap())
        .unwrap();
    assert_eq!(document.input.queued, ["/help", "queued follow-up"]);
    assert_eq!(document.input.draft.text, "draft kept");
    assert_eq!(document.effort, Effort::High);
    std::fs::write(directory.path().join("RESUMING.txt"), &document.id).unwrap();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    resume(agent, tests::config(&directory), None).unwrap();
    assert_eq!(fixture.finish().len(), 3);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("executions.txt")).unwrap(),
        "once\n"
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
        .expect("use /export after resuming");
    let exported = std::fs::read_to_string(export.path()).unwrap();
    assert!(exported.contains("Saved answer"));
    assert!(exported.contains("saved tool output"));
    assert!(exported.contains("partial second answer"));
    assert!(exported.contains("/resume"));
    assert!(!exported.contains("isolated-fixture-key"));
}
