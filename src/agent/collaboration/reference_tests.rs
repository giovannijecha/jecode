//! References resolve across one conversation tree: a collaborator reads the
//! outputs and temporary files its main agent hands it, the main agent reads
//! the outputs its collaborators report, and other conversations stay out.

use super::*;
use crate::{backend::openrouter::OpenRouter, test_support::Directory};

const OFFLINE: &str = "http://127.0.0.1:9/chat/completions";

fn main_agent(project: &Directory, home: &Directory) -> Agent {
    let mut agent = Agent::new(
        OpenRouter::fixture(OFFLINE.into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
}

/// The saved stdout reference of a command that prints `text`.
fn saved(agent: &Agent, text: &str) -> String {
    let result = agent.tools.execute(
        "bash",
        &Value::object([("command", Value::string(format!("printf '{text}'")))]).encode(),
    );
    result
        .get("stdout_file")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{result:?}"))
        .to_owned()
}

pub(super) fn read(agent: &Agent, reference: &str) -> Value {
    agent.tools.execute(
        "read",
        &Value::object([("path", Value::string(reference))]).encode(),
    )
}

pub(super) fn assert_reads(agent: &Agent, reference: &str, text: &str) {
    let result = read(agent, reference);
    assert!(result.get("error").is_none(), "{reference}: {result:?}");
    assert!(result.encode().contains(text), "{reference}: {result:?}");
}

pub(super) fn refusal(agent: &Agent, reference: &str) -> String {
    let result = read(agent, reference);
    result
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{reference} was readable: {result:?}"))
        .to_owned()
}

#[test]
fn collaborators_read_outputs_their_main_agent_hands_them() {
    let project = Directory::new();
    let copy = Directory::new();
    let home = Directory::new();
    let parent = main_agent(&project, &home);
    let reference = saved(&parent, "main log");
    let shared = parent
        .new_child("a1", project.path(), Mode::Shared)
        .unwrap();
    let isolated = parent.new_child("a2", copy.path(), Mode::Isolated).unwrap();
    assert_reads(&shared, &reference, "main log");
    assert_reads(&isolated, &reference, "main log");
    // Paged reads follow the same route.
    let page = shared.tools.execute(
        "read",
        &Value::object([
            ("path", Value::string(&reference)),
            ("byte_offset", Value::number(5)),
        ])
        .encode(),
    );
    assert!(page.encode().contains("log"), "{page:?}");
}

#[test]
fn main_agent_reads_outputs_its_collaborators_report() {
    let project = Directory::new();
    let copy = Directory::new();
    let home = Directory::new();
    let parent = main_agent(&project, &home);
    let shared = parent
        .new_child("a1", project.path(), Mode::Shared)
        .unwrap();
    let isolated = parent.new_child("a2", copy.path(), Mode::Isolated).unwrap();
    let from_shared = saved(&shared, "shared child log");
    let from_isolated = saved(&isolated, "isolated child log");
    assert_reads(&parent, &from_shared, "shared child log");
    assert_reads(&parent, &from_isolated, "isolated child log");
    // Each collaborator still reads its own outputs.
    assert_reads(&isolated, &from_isolated, "isolated child log");
}

#[test]
fn outputs_of_another_conversation_are_refused() {
    let project = Directory::new();
    let home = Directory::new();
    let first = main_agent(&project, &home);
    let second = main_agent(&project, &home);
    let first_child = first.new_child("a1", project.path(), Mode::Shared).unwrap();
    let second_child = second
        .new_child("a1", project.path(), Mode::Shared)
        .unwrap();
    let other_main = saved(&second, "other main");
    let other_child = saved(&second_child, "other child");
    for reference in [&other_main, &other_child] {
        let error = refusal(&first_child, reference);
        assert!(error.contains("not saved for this conversation"), "{error}");
        assert!(!error.contains('\n'), "{error}");
    }
    let error = refusal(&first, &other_child);
    assert!(error.contains("not saved for this conversation"), "{error}");
    // A removed output says so instead of blaming the conversation.
    let own = saved(&first, "own");
    let session = first.sessions().unwrap().id();
    let removed = own.replace(
        &format!("output:{session}:"),
        &format!("output:{session}:1-1-"),
    );
    let error = refusal(&first_child, &removed);
    assert!(error.contains("no longer saved"), "{error}");
}

#[test]
fn resumed_conversation_still_resolves_its_tree() {
    let project = Directory::new();
    let home = Directory::new();
    let original = main_agent(&project, &home);
    let main_reference = saved(&original, "before resume");
    let child = original
        .new_child("a1", project.path(), Mode::Shared)
        .unwrap();
    let child_reference = saved(&child, "child before resume");
    let id = original.sessions().unwrap().id();
    // A conversation is saved once it has a request.
    original.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Run the checks")),
    ]));
    original.save_session().unwrap();
    drop(child);
    drop(original);

    let mut resumed = main_agent(&project, &home);
    resumed.resume(&id).unwrap();
    assert_reads(&resumed, &main_reference, "before resume");
    assert_reads(&resumed, &child_reference, "child before resume");
    let later = resumed
        .new_child("a2", project.path(), Mode::Shared)
        .unwrap();
    assert_reads(&later, &main_reference, "before resume");
}

pub(super) fn call(agent: &Agent, name: &str, fields: &[(&str, &str)]) -> Value {
    let arguments = Value::Object(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), Value::string(*value)))
            .collect(),
    );
    agent.tools.execute(name, &arguments.encode())
}

#[test]
fn collaborators_read_temporary_files_their_main_agent_hands_them() {
    let project = Directory::new();
    let home = Directory::new();
    let parent = main_agent(&project, &home);
    let unrelated = main_agent(&project, &home);
    for (agent, text) in [(&parent, "main notes"), (&unrelated, "other notes")] {
        let written = call(
            agent,
            "write",
            &[("path", "tmp:handoff.txt"), ("content", text)],
        );
        assert!(written.get("error").is_none(), "{written:?}");
    }
    let child = parent
        .new_child("a1", project.path(), Mode::Shared)
        .unwrap();
    assert_reads(&child, "tmp:handoff.txt", "main notes");

    // Changes stay in the collaborator's own area.
    let edited = child.tools.execute(
        "edit",
        &Value::object([
            ("path", Value::string("tmp:handoff.txt")),
            (
                "edits",
                Value::Array(vec![Value::object([
                    ("old_text", Value::string("main")),
                    ("new_text", Value::string("child")),
                ])]),
            ),
        ])
        .encode(),
    );
    assert!(edited.get("error").is_some(), "{edited:?}");
    let written = call(
        &child,
        "write",
        &[("path", "tmp:handoff.txt"), ("content", "child notes")],
    );
    assert!(written.get("error").is_none(), "{written:?}");
    assert_reads(&child, "tmp:handoff.txt", "child notes");
    assert_reads(&parent, "tmp:handoff.txt", "main notes");

    // Another conversation's collaborator never sees this conversation's files.
    let other_child = unrelated
        .new_child("a1", project.path(), Mode::Shared)
        .unwrap();
    assert_reads(&other_child, "tmp:handoff.txt", "other notes");
    assert!(read(&child, "tmp:missing.txt").get("error").is_some());
}
