use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::config::{Settings, Store};
use crate::json::Value;
use crate::test_support::{Directory, HttpFixture};
use crate::tools::Tools;
use std::io::Cursor;

fn response(text: &str) -> Value {
    Value::object([(
        "choices",
        Value::Array(vec![Value::object([
            ("finish_reason", Value::string("stop")),
            (
                "message",
                Value::object([
                    ("role", Value::string("assistant")),
                    ("content", Value::string(text)),
                ]),
            ),
        ])]),
    )])
}

#[test]
fn failed_plain_direct_write_records_only_a_safe_command_label() {
    let home = Directory::new();
    let project = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    let mut config = SessionConfig {
        store: Store::new(home.path().to_path_buf()),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    let mut output = Vec::new();
    let mut status = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut Cursor::new(
            "/terminal write missing SECRET_DIRECT_INPUT_42\n/terminal write SECRET_AS_ID\n/exit\n",
        ),
        &mut output,
        &mut status,
        Vec::new(),
    )
    .unwrap();
    let archive = format!(
        "{:?}{:?}",
        agent.archive().messages.lock().unwrap(),
        agent.archive().events.lock().unwrap()
    );
    assert!(!archive.contains("SECRET_DIRECT_INPUT_42"));
    assert!(!archive.contains("SECRET_AS_ID"));
    assert!(archive.contains("/terminal write [direct input]"));
    let status = String::from_utf8(status).unwrap();
    assert!(!status.contains("SECRET_DIRECT_INPUT_42"));
    assert!(!status.contains("SECRET_AS_ID"));
}

#[test]
fn backend_command_keeps_the_conversation_and_saved_default() {
    let home = Directory::new();
    let project = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    let mut config = SessionConfig {
        store: Store::new(home.path().to_path_buf()),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/default".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    let before = agent.archive().messages.lock().unwrap().len();
    let mut output = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut Cursor::new("/backend openrouter fixture/new\n/exit\n"),
        &mut output,
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(agent.backend_id(), "openrouter");
    assert_eq!(agent.model(), "fixture/new");
    assert_eq!(config.settings.model, "fixture/default");
    assert!(agent.archive().messages.lock().unwrap().len() >= before);
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Backend set to openrouter / fixture/new")
    );
}

#[test]
fn skills_command_lists_local_catalog_and_skips_malformed_entries_without_an_api_request() {
    let home = Directory::new();
    let project = Directory::new();
    let skills = home.path().join("skills");
    std::fs::create_dir_all(skills.join("review")).unwrap();
    std::fs::create_dir_all(skills.join("broken")).unwrap();
    std::fs::write(
        skills.join("review/SKILL.md"),
        "---\nname: review\ndescription: Inspect a change.\n---\n\n# Review\n",
    )
    .unwrap();
    std::fs::write(skills.join("broken/SKILL.md"), "missing frontmatter").unwrap();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    let mut config = SessionConfig {
        store: Store::new(home.path().to_path_buf()),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    let mut output = Vec::new();
    let mut status = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut Cursor::new("/skills\n/skills extra\n/exit\n"),
        &mut output,
        &mut status,
        Vec::new(),
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("1 available skill · 1 skipped"), "{output}");
    assert!(output.contains("review  Inspect a change."), "{output}");
    assert!(output.contains("Skipped"), "{output}");
    assert!(output.contains("Unknown command"), "{output}");
    assert!(status.is_empty(), "{}", String::from_utf8_lossy(&status));
    assert!(
        agent
            .archive()
            .messages
            .lock()
            .unwrap()
            .iter()
            .all(|message| { message.get("role").and_then(Value::as_str) == Some("system") })
    );
}

#[test]
fn skills_command_explains_an_empty_catalog() {
    let home = Directory::new();
    let project = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    let mut config = SessionConfig {
        store: Store::new(home.path().to_path_buf()),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    let mut output = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut Cursor::new("/skills\n/exit\n"),
        &mut output,
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("No skills installed.")
    );
}

#[test]
fn interactive_session_recovers_from_api_errors_and_can_clear_history() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            401,
            Value::object([(
                "error",
                Value::object([("message", Value::string("Invalid fixture credential"))]),
            )]),
        ),
        (200, response("Recovered.")),
        (200, response("Fresh conversation.")),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut input =
        Cursor::new("First question\nTry again\n/export\n/clear\nFresh question\n/export\n/exit\n");
    let mut output = Vec::new();
    let mut events = Vec::new();
    let mut config = SessionConfig {
        store: Store::new(directory.path().join(".jecode")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    chat(
        &mut agent,
        &mut config,
        &mut input,
        &mut output,
        &mut events,
        Vec::new(),
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Recovered."));
    assert!(output.contains("Conversation cleared."));
    assert!(output.contains("Fresh conversation."));
    assert!(output.contains("Exported conversation to"));
    let exports: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("JECODE-SESSION-")
        })
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .collect();
    assert_eq!(exports.len(), 2);
    assert!(
        exports
            .iter()
            .any(|text| text.contains("Recovered.") && !text.contains("Fresh conversation."))
    );
    assert!(
        exports
            .iter()
            .any(|text| text.contains("Fresh conversation.") && !text.contains("First question"))
    );
    assert!(String::from_utf8(events).unwrap().contains("HTTP 401"));
    let requests = fixture.finish();
    let messages = requests[2]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[1].get("content").unwrap().as_str(),
        Some("Fresh question")
    );
}

#[test]
fn model_command_preserves_context_and_does_not_change_saved_defaults() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, response("Before.")), (200, response("After."))]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut config = SessionConfig {
        store: Store::new(directory.path().join(".jecode")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    let mut input =
        Cursor::new("First task\n/help\n/unknown\n/model fixture/new\nSecond task\n/exit\n");
    let mut output = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut input,
        &mut output,
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(agent.model(), "fixture/new");
    assert!(config.store.load().unwrap().is_none());
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Unknown command"));
    assert!(output.contains("Model set to fixture/new"));
    let requests = fixture.finish();
    assert_eq!(
        requests[1].body.get("model").and_then(Value::as_str),
        Some("fixture/new")
    );
    assert_eq!(
        requests[1]
            .body
            .get("messages")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn routing_command_offers_the_shared_choices_and_applies_from_the_next_request() {
    let directory = Directory::new();
    let listing = crate::json::parse(
        r#"{"data":{"endpoints":[{"tag":"one/fp8","supported_parameters":["tools"],
        "pricing":{"prompt":"0.0000003","completion":"0.0000012"}},
        {"tag":"notools/x","supported_parameters":[]}]}}"#,
    )
    .unwrap();
    let fixture = HttpFixture::new(vec![(200, listing), (200, response("Pinned."))]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.set_routing(crate::backend::openrouter::Routing::Auto);
    let mut config = SessionConfig {
        store: Store::new(directory.path().join(".jecode")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    let mut input = Cursor::new("/routing\n9\n3\n2\nTask\n/exit\n");
    let mut output = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut input,
        &mut output,
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        agent.routing(),
        crate::backend::openrouter::Routing::Endpoint("one/fp8".into())
    );
    assert!(config.store.load().unwrap().is_none());
    let output = String::from_utf8(output).unwrap();
    for expected in [
        "Routing for fixture/model",
        "1. Auto — Best balance · current · native routing with fallbacks",
        "2. one/fp8 · $0.3/$1.2",
        "3. notools/x · no tools",
        "Choose one of the displayed numbers.",
        "notools/x does not accept tool calls, which every Jecode request includes; choose another.",
        "Pinned to one/fp8 without fallbacks",
        "Routing set to Endpoint one/fp8 · was Auto — Best balance · applies from the next request",
    ] {
        assert!(output.contains(expected), "missing {expected}:\n{output}");
    }
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].headers[0].starts_with("GET /models/fixture/model/endpoints "));
    assert_eq!(
        requests[1]
            .body
            .get("provider")
            .map(Value::encode)
            .as_deref(),
        Some(r#"{"allow_fallbacks":false,"order":["one/fp8"]}"#)
    );
}

#[test]
fn plain_quit_alias_is_local_and_accepts_mixed_case() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut config = SessionConfig {
        store: Store::new(directory.path().join(".jecode")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    let mut output = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut Cursor::new("/HeLp\n/QuIt\nThis must not be sent\n"),
        &mut output,
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("/init"));
    assert!(output.contains("/exit"));
    assert!(!output.contains("/quit"));
    assert!(fixture.finish().is_empty());
}

#[test]
fn init_runs_a_model_tool_turn_for_root_project_instructions() {
    let directory = Directory::new();
    let path = directory.path().join("AGENTS.md");
    std::fs::write(&path, "# Existing\n\nKeep this verified rule.\n").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            crate::test_support::completion(
                "I checked the existing project instructions.",
                vec![crate::test_support::tool_call(
                    "update-instructions",
                    "write",
                    Value::object([
                        ("path", Value::string("AGENTS.md")),
                        (
                            "content",
                            Value::string(
                                "# Existing\n\nKeep this verified rule.\n\n# Commands\n\nRun cargo test.\n",
                            ),
                        ),
                    ]),
                )],
            ),
        ),
        (200, response("Updated root AGENTS.md.")),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut config = SessionConfig {
        store: Store::new(directory.path().join(".jecode")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    let mut output = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut Cursor::new("/InIt\n/exit\n"),
        &mut output,
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("Keep this verified rule.")
    );
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Updated root AGENTS.md.")
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    let messages = requests[0]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    let request = messages
        .last()
        .unwrap()
        .get("content")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(request.starts_with("/init\n\n"));
    assert!(request.contains("update only the root AGENTS.md"));
}
