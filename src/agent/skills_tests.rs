use super::*;
use crate::test_support::Directory;
use std::fs;

fn fixture() -> (Directory, Directory, Agent) {
    let home = Directory::new();
    let project = Directory::new();
    let package = home.path().join("skills/audit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("SKILL.md"),
        "---\nname: audit\ndescription: Audit a change.\n---\n\n# Audit\nCheck carefully.\n",
    )
    .unwrap();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    (home, project, agent)
}

fn skill_call(path: &str) -> ToolCall {
    ToolCall {
        id: "read-skill".into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string(path))]).encode(),
    }
}

#[test]
fn catalog_refresh_invalidates_measurement_and_exposes_current_metadata() {
    let (home, _project, mut agent) = fixture();
    assert_eq!(agent.skills().unwrap()[0].name, "audit");
    agent.context.input_tokens = Some(10);
    agent.context.measured_end = 1;
    fs::write(
        home.path().join("skills/audit/SKILL.md"),
        "---\nname: audit\ndescription: Review a project.\n---\n\n# Audit\n",
    )
    .unwrap();
    agent.prepare_turn("Inspect").unwrap();
    assert_eq!(agent.skills.list()[0].description, "Review a project.");
    assert_eq!(agent.context.input_tokens, None);
    assert_eq!(agent.context.measured_end, 0);
}

#[test]
fn a_read_skill_receipt_remains_reloadable_after_compaction_and_resume() {
    let (home, project, mut first) = fixture();
    first.prepare_turn("Inspect code").unwrap();
    let call = skill_call("skill:audit");
    first.messages.lock().unwrap().push(Value::object([
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
    let page = first.execute_tool(&call);
    assert!(page.get("error").is_none(), "{}", page.encode());
    assert!(
        page.get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("Check carefully")
    );
    first.tool_result(&call, &page);
    let original_message_count = first.messages.lock().unwrap().len();
    first.context.from = original_message_count;
    first.context.summary = "Continue auditing the project.".into();
    assert!(first.skill_instructions().contains("skill:audit"));
    first.save_session().unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);

    let mut resumed = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    resumed.prepare_turn("Continue").unwrap();
    let instructions = resumed.skill_instructions();
    assert!(instructions.contains("Skills read in this conversation"));
    assert!(instructions.contains("skill:audit"));
    let page = resumed.execute_tool(&skill_call("skill:audit"));
    assert!(page.get("error").is_none(), "{}", page.encode());
}

#[test]
fn skill_references_cannot_be_written_through_file_tools() {
    let (_home, _project, agent) = fixture();
    let result = agent.execute_tool(&ToolCall {
        id: "deny".into(),
        name: "write".into(),
        arguments: Value::object([
            ("path", Value::string("skill:audit/SKILL.md")),
            ("content", Value::string("changed")),
        ])
        .encode(),
    });
    assert!(
        result
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("read-only")
    );
}
