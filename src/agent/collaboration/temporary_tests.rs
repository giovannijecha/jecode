//! A main agent reads the temporary files its collaborators write, read-only
//! and only within its own conversation.

use super::recovery_tests::{action, create};
use super::reference_tests::{assert_reads, call, refusal};
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    cancel::Cancellation,
    test_support::{Directory, HttpFixture, completion, tool_call},
};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(60);

/// A main agent whose collaborators, in the order they start, each write
/// one tmp: file through their own tools, with the fixture serving them.
fn main_agent(
    project: &Directory,
    home: &Directory,
    writes: &[(&str, &str)],
) -> (Agent, HttpFixture) {
    let fixture = writes
        .iter()
        .flat_map(|(alias, text)| {
            let arguments = Value::object([
                ("path", Value::string(format!("tmp:{alias}"))),
                ("content", Value::string(*text)),
            ]);
            [
                (
                    200,
                    completion("", vec![tool_call("save", "write", arguments)]),
                ),
                (200, completion("Saved", vec![])),
            ]
        })
        .collect();
    let fixture = HttpFixture::new(fixture);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    (agent, fixture)
}

/// Starts the next collaborator and waits until it has written its file.
fn delegate(agent: &Agent, id: &str) {
    agent.agent_tool(&create("shared")).unwrap();
    let records = agent
        .collaboration()
        .wait(Some(id), TIMEOUT, &Cancellation::default())
        .unwrap();
    assert_eq!(
        records[0].get("state").and_then(Value::as_str),
        Some("returned"),
        "{records:?}"
    );
}

fn error(result: &Value) -> &str {
    result
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{result:?}"))
}

#[test]
fn main_agent_reads_a_file_one_collaborator_wrote_and_writes_its_own() {
    let project = Directory::new();
    let home = Directory::new();
    let (parent, fixture) = main_agent(&project, &home, &[("findings.md", "child findings")]);
    delegate(&parent, "a1");
    fixture.finish();
    assert_reads(&parent, "tmp:findings.md", "child findings");
    assert_reads(&parent, "tmp:a1:findings.md", "child findings");

    // Edits and writes stay in the main agent's own area.
    let edited = call(
        &parent,
        "edit",
        &[
            ("path", "tmp:findings.md"),
            ("old_text", "child"),
            ("new_text", "main"),
        ],
    );
    assert!(
        error(&edited).contains("Could not inspect temporary path"),
        "{edited:?}"
    );
    let refused = call(
        &parent,
        "write",
        &[("path", "tmp:a1:findings.md"), ("content", "main")],
    );
    assert_eq!(
        error(&refused),
        "tmp:a1:findings.md is a collaborator's temporary file and read-only; write tmp:findings.md in your own area"
    );
    let written = call(
        &parent,
        "write",
        &[("path", "tmp:findings.md"), ("content", "main findings")],
    );
    assert!(written.get("error").is_none(), "{written:?}");
    assert_reads(&parent, "tmp:findings.md", "main findings");
    assert_reads(&parent, "tmp:a1:findings.md", "child findings");
}

#[test]
fn a_path_several_collaborators_wrote_names_them() {
    let project = Directory::new();
    let home = Directory::new();
    let (parent, fixture) = main_agent(
        &project,
        &home,
        &[("notes.md", "first notes"), ("notes.md", "second notes")],
    );
    delegate(&parent, "a1");
    delegate(&parent, "a2");
    fixture.finish();
    assert_eq!(
        refusal(&parent, "tmp:notes.md"),
        "tmp:notes.md is not in your temporary area but in those of agents a1, a2; read one as tmp:a1:notes.md"
    );
    assert_reads(&parent, "tmp:a1:notes.md", "first notes");
    assert_reads(&parent, "tmp:a2:notes.md", "second notes");
}

#[test]
fn collaborators_of_another_conversation_are_refused() {
    let project = Directory::new();
    let home = Directory::new();
    let (other, other_fixture) = main_agent(&project, &home, &[("notes.md", "other notes")]);
    delegate(&other, "a1");
    other_fixture.finish();
    let (parent, fixture) = main_agent(&project, &home, &[("own.md", "own notes")]);
    delegate(&parent, "a1");
    fixture.finish();

    let missing = refusal(&parent, "tmp:notes.md");
    assert!(
        missing.contains("Could not inspect temporary path"),
        "{missing}"
    );
    assert_eq!(
        refusal(&parent, "tmp:a1:notes.md"),
        "No agent a1 of this conversation has tmp:notes.md"
    );
    assert_eq!(
        refusal(&parent, "tmp:a2:own.md"),
        "No agent a2 of this conversation has tmp:own.md"
    );
    assert_reads(&parent, "tmp:own.md", "own notes");
    assert_reads(&other, "tmp:notes.md", "other notes");
}

#[test]
fn resumed_conversation_reads_its_closed_collaborators_files() {
    let project = Directory::new();
    let home = Directory::new();
    let (parent, fixture) = main_agent(&project, &home, &[("notes.md", "child notes")]);
    delegate(&parent, "a1");
    fixture.finish();
    // A closed collaborator's record survives resume only in saved receipts.
    parent.agent_tool(&action("close")).unwrap();
    parent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Investigate")),
    ]));
    parent.save_session().unwrap();
    let id = parent.sessions().unwrap().id();
    drop(parent);

    let (mut resumed, _idle) = main_agent(&project, &home, &[]);
    resumed.resume(&id).unwrap();
    assert_reads(&resumed, "tmp:notes.md", "child notes");
    assert_reads(&resumed, "tmp:a1:notes.md", "child notes");
}
