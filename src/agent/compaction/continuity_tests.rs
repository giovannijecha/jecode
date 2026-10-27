use super::super::*;
use crate::backend::openrouter::Limits;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};

fn message(role: &str, text: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(text)),
    ])
}

fn history(agent: &mut Agent, older: &str) -> Vec<Value> {
    agent
        .prepare_turn("Inspect source; preserve protected.txt.")
        .unwrap();
    agent.messages.lock().unwrap().extend([
        message("assistant", older),
        message(
            "user",
            "Check the dates next; keep the original constraints.",
        ),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "recent",
                    "read",
                    Value::object([("path", Value::string("source"))]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("recent")),
            (
                "content",
                Value::string(
                    Value::object([("content", Value::string("LATEST EXACT RESULT Ω 日本"))])
                        .encode(),
                ),
            ),
        ]),
    ]);
    agent.messages.lock().unwrap().clone()
}

#[test]
fn compaction_keeps_the_recent_request_and_complete_tool_pair_after_resume() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        completion("Original goal and earlier findings preserved.", vec![]),
    )]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let original = history(&mut agent, &"Earlier findings ".repeat(600));
    // Reserve room for the retained exchange and summary independently of
    // native prompt/catalog wording. This test checks continuity, not a 24K limit.
    let fixed = agent.current_system_message().encode().len()
        + crate::tools::encoded_size()
        + agent.request_environment().len();
    let capacity = (fixed + 8192).div_ceil(4) * 5;
    agent
        .compact_if_needed(
            Some(Limits {
                context: capacity,
                output: Some(4096),
            }),
            true,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(agent.context.from, 3);
    assert_eq!(*agent.messages.lock().unwrap(), original);
    let projected = agent.transport_messages();
    assert!(projected.ends_with(&original[3..]));
    assert!(projected[1].encode().contains("preserve protected.txt"));
    agent.checkpoint(crate::sessions::Stage::Ready).unwrap();
    let id = agent.sessions().unwrap().id();
    drop(agent);
    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.transport_messages(), projected);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].body.encode().contains("LATEST EXACT RESULT"));
}

#[test]
fn a_tool_pair_larger_than_the_tail_budget_is_archived_together() {
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    let original = history(&mut agent, "Old findings");
    // The tool result fits but its complete originating call does not.
    let result_size = crate::context::bytes(&original[5..]);
    assert_eq!(
        agent.recent_boundary(&original, result_size + 1),
        original.len()
    );
}

#[test]
fn expired_tool_view_does_not_shrink_the_recent_tail() {
    let directory = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut image = crate::attachments::tests::attachment("att-1-1-1", "shot.png", "image/png");
    image.width = Some(4);
    image.height = Some(3);
    let receipt = Value::object([
        ("view", Value::string("image")),
        ("attachment", image.value()),
    ]);
    let messages = vec![
        message("system", "Agent"),
        message("user", "Inspect the image"),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "earlier",
                    "read",
                    Value::object([("path", Value::string("image"))]),
                )]),
            ),
        ]),
        message("tool", &receipt.encode()),
        message("assistant", "The first image is clear"),
        message("user", "Compare it with the next image"),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    "recent",
                    "read",
                    Value::object([("path", Value::string("next image"))]),
                )]),
            ),
        ]),
        message("tool", &receipt.encode()),
    ];
    let budget = crate::context::bytes_for(&messages[2..], agent.client.inputs());
    assert_eq!(agent.recent_boundary(&messages, budget), 2);
}

#[test]
fn a_failure_in_a_later_summary_portion_keeps_the_previous_context_and_originals() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("Partial summary only.", vec![])),
        (
            401,
            Value::object([(
                "error",
                Value::object([("message", Value::string("Fixture credential rejection"))]),
            )]),
        ),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let original = history(&mut agent, &"Earlier findings ".repeat(4000));
    // Leave room for native instructions and the retained exchange while
    // keeping the transcript large enough to require a second portion.
    let fixed = agent.current_system_message().encode().len()
        + crate::tools::encoded_size()
        + agent.request_environment().len();
    let capacity = (fixed + 8192).div_ceil(4) * 5;
    let previous = agent.context.value();
    let result = agent.compact_if_needed(
        Some(Limits {
            context: capacity,
            output: Some(4096),
        }),
        true,
        &mut |_| Ok(()),
    );
    assert!(
        result.unwrap_err().contains("Fixture credential rejection"),
        "compaction must reach the second summary portion"
    );
    assert_eq!(agent.context.value(), previous);
    assert_eq!(*agent.messages.lock().unwrap(), original);
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn compaction_saves_skill_references_even_when_the_model_summary_omits_them() {
    let home = Directory::new();
    let project = Directory::new();
    let package = home.path().join("skills/audit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("SKILL.md"),
        "---\nname: audit\ndescription: Audit a change.\n---\nCheck carefully.\n",
    )
    .unwrap();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let http = crate::test_support::HttpFixture::new(vec![(
        200,
        crate::test_support::completion("Earlier work summarized; continue checking.", vec![]),
    )]);
    agent.replace_client(OpenRouter::fixture(http.endpoint.clone()));
    agent.prepare_turn("Inspect code").unwrap();
    let system = agent.current_system_message();
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        (
            "tool_calls",
            Value::Array(vec![crate::test_support::tool_call(
                "read-skill",
                "read",
                Value::object([("path", Value::string("skill:audit"))]),
            )]),
        ),
    ]));
    let call = ToolCall {
        id: "read-skill".into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string("skill:audit"))]).encode(),
    };
    let result = agent.execute_tool(&call);
    agent.tool_result(&call, &result);
    let capacity = agent.fixture_capacity(8_192, 80);
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("assistant")),
            (
                "content",
                Value::string("Earlier inspection notes. ".repeat(400)),
            ),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Now check the report.")),
        ]),
    ]);
    agent
        .compact_if_needed(
            Some(crate::backend::openrouter::Limits {
                context: capacity,
                output: Some(4096),
            }),
            true,
            &mut |_| Ok(()),
        )
        .unwrap();
    assert!(agent.context.skill_references.contains("skill:audit"));
    assert!(!agent.context.summary.contains("skill:audit"));
    assert!(agent.transport_messages().iter().any(|message| {
        message
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("Skills read in this conversation"))
    }));
    assert_eq!(agent.current_system_message(), system);
    agent.checkpoint(crate::sessions::Stage::Ready).unwrap();
    let id = agent.sessions().unwrap().id();
    let references = agent.context.skill_references.clone();
    drop(agent);
    let mut resumed = Agent::new(
        OpenRouter::fixture(http.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.context.skill_references, references);
    assert_eq!(http.finish().len(), 1);
}
