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

#[test]
fn remembering_a_call_covers_its_outside_files_one_by_one() {
    let project = Directory::new();
    let other = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("a.rs"), "before\n").unwrap();
    let outside: Vec<String> = ["b.rs", "c.rs", "d.rs"]
        .iter()
        .map(|name| {
            let path = other.path().join(name);
            fs::write(&path, "before\nbefore\n").unwrap();
            path.to_string_lossy().into_owned()
        })
        .collect();
    let single = tool_call(
        "single",
        "edit",
        Value::object([
            ("path", Value::string(&outside[0])),
            ("old_text", Value::string("before")),
            ("new_text", Value::string("after")),
            ("replace_all", Value::Bool(true)),
        ]),
    );
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion("", vec![edit("first", &["a.rs", &outside[0], &outside[1]])]),
        ),
        (200, completion("", vec![single])),
        (200, completion("", vec![edit("reordered", &[&outside[1]])])),
        (
            200,
            completion("", vec![edit("wider", &[&outside[1], &outside[2]])]),
        ),
        (200, completion("Done", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.permissions().set_surface(Surface::Panel);
    let permissions = agent.permissions();
    let user = thread::spawn(move || {
        let mut seen = Vec::new();
        for _ in 0..400 {
            if let Some((request, _)) = permissions.pending() {
                let decision = if seen.is_empty() {
                    Decision::Remember
                } else {
                    Decision::Once
                };
                seen.push(request.call_id.clone());
                permissions.decide(request.id, decision).unwrap();
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        seen
    });
    agent.run_turn("Change them", &mut |_| Ok(())).unwrap();
    // The later edits of remembered files ask nothing; only a new file does.
    assert_eq!(user.join().unwrap(), ["first", "wider"]);
    // Each outside file is remembered and revocable on its own, not the project file.
    let grants = agent.permissions().grants();
    let targets: Vec<&str> = grants.iter().map(|grant| grant.target.as_str()).collect();
    assert_eq!(targets.len(), 2);
    assert!(targets[0].ends_with("b.rs") && targets[1].ends_with("c.rs"));

    agent.save_session().unwrap();
    let id = agent.sessions().unwrap().id();
    drop(agent);
    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.permissions().grants(), grants);
    assert_eq!(fixture.finish().len(), 5);
}

#[test]
fn json_text_files_from_the_model_run_and_are_stored_decoded() {
    let project = Directory::new();
    for name in ["a.rs", "b.rs"] {
        fs::write(project.path().join(name), "before\n").unwrap();
    }
    let decoded = edit("text", &["a.rs", "b.rs"]);
    let files = decoded
        .get("function")
        .and_then(|function| function.get("arguments"))
        .and_then(Value::as_str)
        .map(|arguments| crate::json::parse(arguments).unwrap())
        .and_then(|arguments| arguments.get("files").cloned())
        .unwrap();
    let as_text = tool_call(
        "text",
        "edit",
        Value::object([("files", Value::string(files.encode()))]),
    );
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![as_text])),
        (200, completion("Done", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.permissions().set_level(Level::Workspace).unwrap();
    agent.run_turn("Rename it", &mut |_| Ok(())).unwrap();
    for name in ["a.rs", "b.rs"] {
        assert_eq!(
            fs::read_to_string(project.path().join(name)).unwrap(),
            "after\n"
        );
    }
    // History keeps the decoded call, so later requests and cards agree.
    let archive = agent.archive();
    let messages = archive.messages.lock().unwrap();
    let call = messages
        .iter()
        .filter_map(|message| message.get("tool_calls").and_then(Value::as_array))
        .flatten()
        .map(|call| crate::conversation::ToolCall::parse(call).unwrap())
        .find(|call| call.id == "text")
        .unwrap();
    let arguments = crate::json::parse(&call.arguments).unwrap();
    assert!(matches!(arguments.get("files"), Some(Value::Array(_))));
    drop(messages);
    assert_eq!(fixture.finish().len(), 2);
}
