//! A multi-file edit is one call for permissions: one refusal or one request
//! names every file.
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    permissions::{Decision, Level, Surface},
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::{fs, thread};

fn edit(id: &str, paths: &[&str]) -> Value {
    let files = paths
        .iter()
        .map(|path| {
            Value::object([
                ("path", Value::string(*path)),
                (
                    "edits",
                    Value::Array(vec![Value::object([
                        ("old_text", Value::string("before")),
                        ("new_text", Value::string("after")),
                    ])]),
                ),
            ])
        })
        .collect();
    tool_call(id, "edit", Value::object([("files", Value::Array(files))]))
}

fn result(agent: &Agent, id: &str) -> Value {
    let archive = agent.archive();
    let messages = archive.messages.lock().unwrap();
    let content = messages
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    crate::json::parse(&content).unwrap()
}

#[test]
fn read_level_refuses_the_whole_call_naming_every_file() {
    let project = Directory::new();
    for name in ["a.rs", "b.rs"] {
        fs::write(project.path().join(name), "before\n").unwrap();
    }
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![edit("both", &["a.rs", "b.rs"])])),
        (200, completion("Read only", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.permissions().set_level(Level::Read).unwrap();
    agent.run_turn("Rename it", &mut |_| Ok(())).unwrap();
    let refused = result(&agent, "both");
    let reason = refused.get("reason").and_then(Value::as_str).unwrap();
    assert!(
        reason.contains("Read permission level: editing "),
        "{reason}"
    );
    assert!(
        reason.contains("a.rs, ") && reason.contains("b.rs. "),
        "{reason}"
    );
    for name in ["a.rs", "b.rs"] {
        assert_eq!(
            fs::read_to_string(project.path().join(name)).unwrap(),
            "before\n"
        );
    }
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn workspace_asks_once_for_a_call_with_an_outside_file() {
    let project = Directory::new();
    let other = Directory::new();
    fs::write(project.path().join("a.rs"), "before\n").unwrap();
    let outside = other.path().join("b.rs");
    fs::write(&outside, "before\n").unwrap();
    let outside_path = outside.to_string_lossy().into_owned();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion("", vec![edit("both", &["a.rs", &outside_path])]),
        ),
        (200, completion("Done", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    assert_eq!(agent.permissions().level(), Level::Workspace);
    agent.permissions().set_surface(Surface::Panel);
    let permissions = agent.permissions();
    let user = thread::spawn(move || {
        let mut decided = 0;
        for _ in 0..400 {
            if let Some((request, _)) = permissions.pending() {
                permissions.decide(request.id, Decision::Once).unwrap();
                decided += 1;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        decided
    });
    let mut targets = Vec::new();
    agent
        .run_turn("Change both", &mut |event| {
            if let Event::PermissionRequest { request, .. } = event {
                targets.push(request.access.target);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(user.join().unwrap(), 1);
    assert_eq!(targets.len(), 1);
    let (first, second) = targets[0].split_once(", ").unwrap();
    assert!(first.ends_with("a.rs") && second.ends_with("b.rs"));
    let entries = result(&agent, "both");
    let entries = entries.get("files").and_then(Value::as_array).unwrap();
    assert!(entries.iter().all(|entry| entry.get("error").is_none()));
    assert_eq!(fs::read_to_string(&outside).unwrap(), "after\n");
    assert_eq!(
        fs::read_to_string(project.path().join("a.rs")).unwrap(),
        "after\n"
    );
    assert_eq!(fixture.finish().len(), 2);
}
