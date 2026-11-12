//! Permission decisions for an outside path inside one ordered read batch.

use super::*;
use crate::{
    agent::permission::Gate,
    permissions::{Decision, Level, Surface},
    test_support::{HttpFixture, completion, link_directory, tool_call},
};
use std::{
    thread,
    time::{Duration, Instant},
};

fn batch(id: &str, paths: Vec<Value>) -> Value {
    tool_call(id, "read", Value::object([("paths", Value::Array(paths))]))
}

fn result(agent: &Agent, id: &str) -> Value {
    let archive = agent.archive();
    let messages = archive.messages.lock().unwrap();
    let content = messages
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap();
    crate::json::parse(content).unwrap()
}

fn named<'a>(files: &'a [Value], index: usize, path: &str) -> &'a Value {
    assert_eq!(files[index].get("path").and_then(Value::as_str), Some(path));
    &files[index]
}

fn answer_once(agent: &Agent, decision: Decision) -> thread::JoinHandle<bool> {
    let permissions = agent.permissions();
    thread::spawn(move || {
        for _ in 0..400 {
            if let Some((request, _)) = permissions.pending() {
                permissions.decide(request.id, decision).unwrap();
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        false
    })
}

#[test]
fn conflicting_read_arguments_report_their_error_without_requesting_access() {
    let project = Directory::new();
    let outside = Directory::new();
    let file = outside.path().join("outside.txt");
    fs::write(&file, "outside marker\n").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "invalid",
                    "read",
                    Value::object([
                        ("path", Value::string("inside.txt")),
                        (
                            "paths",
                            Value::Array(vec![Value::string(file.to_string_lossy())]),
                        ),
                    ]),
                )],
            ),
        ),
        (200, completion("Understood", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    // No approval surface: an access request would return unavailable.
    agent.permissions().set_surface(Surface::None);
    agent.run_turn("Compare files", &mut |_| Ok(())).unwrap();
    let invalid = result(&agent, "invalid");
    assert_eq!(
        invalid.get("error").and_then(Value::as_str),
        Some("Use path or paths, not both")
    );
    assert!(invalid.get("outcome").is_none());
    assert!(!invalid.encode().contains("outside marker"));
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn full_reads_mixed_outside_and_missing_paths_in_order() {
    let project = Directory::new();
    let other = Directory::new();
    fs::write(project.path().join("inside.txt"), "inside marker\n").unwrap();
    let outside = other.path().join("outside.txt");
    fs::write(&outside, "outside marker\n").unwrap();
    link_directory(other.path(), &project.path().join("alias"));
    let outside = outside.to_string_lossy().into_owned();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![batch(
                    "mixed",
                    vec![
                        Value::string("inside.txt"),
                        Value::string("history:requests"),
                        Value::number(7),
                        Value::string("missing.txt"),
                        Value::string(&outside),
                        Value::string("alias/outside.txt"),
                    ],
                )],
            ),
        ),
        (200, completion("Done", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.permissions().set_level(Level::Full).unwrap();
    agent.run_turn("Compare files", &mut |_| Ok(())).unwrap();
    let result = result(&agent, "mixed");
    let files = result.get("files").and_then(Value::as_array).unwrap();
    assert_eq!(files.len(), 6);
    assert!(
        named(files, 0, "inside.txt")
            .encode()
            .contains("inside marker")
    );
    assert!(named(files, 1, "history:requests").get("error").is_none());
    assert!(files[2].get("error").is_some());
    assert!(named(files, 3, "missing.txt").get("error").is_some());
    assert!(
        named(files, 4, &outside)
            .encode()
            .contains("outside marker")
    );
    assert!(
        named(files, 5, "alias/outside.txt")
            .encode()
            .contains("outside marker")
    );
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn read_level_refuses_a_batch_with_a_link_to_an_outside_file() {
    let project = Directory::new();
    let other = Directory::new();
    fs::write(project.path().join("inside.txt"), "inside marker\n").unwrap();
    fs::write(other.path().join("outside.txt"), "outside marker\n").unwrap();
    link_directory(other.path(), &project.path().join("alias"));
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![batch(
                    "blocked",
                    vec![
                        Value::string("inside.txt"),
                        Value::string("alias/outside.txt"),
                    ],
                )],
            ),
        ),
        (200, completion("Understood", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.permissions().set_level(Level::Read).unwrap();
    agent.run_turn("Compare files", &mut |_| Ok(())).unwrap();
    let refused = result(&agent, "blocked");
    assert_eq!(
        refused.get("outcome").and_then(Value::as_str),
        Some("unavailable")
    );
    assert!(refused.get("files").is_none(), "nothing in the batch ran");
    assert!(!refused.encode().contains("inside marker"));
    assert!(!refused.encode().contains("outside marker"));
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn workspace_asks_once_before_each_outside_batch_and_honors_deny_or_approval() {
    let project = Directory::new();
    let other = Directory::new();
    fs::write(project.path().join("inside.txt"), "inside marker\n").unwrap();
    let outside = other.path().join("outside.txt");
    fs::write(&outside, "outside marker\n").unwrap();
    let outside = outside.to_string_lossy().into_owned();
    let paths = vec![Value::string("inside.txt"), Value::string(&outside)];
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![batch("denied", paths.clone())])),
        (200, completion("Continue", vec![])),
        (200, completion("", vec![batch("approved", paths)])),
        (200, completion("Done", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    assert_eq!(agent.permissions().level(), Level::Workspace);
    agent.permissions().set_surface(Surface::Panel);
    let permissions = agent.permissions();
    let decisions = thread::spawn(move || {
        let mut handled = 0;
        for _ in 0..400 {
            if let Some((request, _)) = permissions.pending() {
                let decision = if handled == 0 {
                    Decision::Deny
                } else {
                    Decision::Once
                };
                permissions.decide(request.id, decision).unwrap();
                handled += 1;
                if handled == 2 {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
        handled
    });
    agent.run_turn("First", &mut |_| Ok(())).unwrap();
    let denied = result(&agent, "denied");
    assert_eq!(
        denied.get("outcome").and_then(Value::as_str),
        Some("denied")
    );
    assert!(denied.get("files").is_none());
    agent.run_turn("Second", &mut |_| Ok(())).unwrap();
    assert_eq!(decisions.join().unwrap(), 2, "one decision per batch");
    let approved = result(&agent, "approved");
    let files = approved.get("files").and_then(Value::as_array).unwrap();
    assert_eq!(files.len(), 2);
    assert!(
        named(files, 0, "inside.txt")
            .encode()
            .contains("inside marker")
    );
    assert!(
        named(files, 1, &outside)
            .encode()
            .contains("outside marker")
    );
    assert_eq!(fixture.finish().len(), 4);
}

#[test]
fn remembered_batch_grants_cover_each_outside_file_but_not_a_new_sibling() {
    let project = Directory::new();
    let other = Directory::new();
    fs::write(project.path().join("inside.txt"), "inside marker\n").unwrap();
    let outside: Vec<_> = ["a.txt", "b.txt", "c.txt"]
        .iter()
        .map(|name| {
            let path = other.path().join(name);
            fs::write(&path, format!("marker-{name}\n")).unwrap();
            path.to_string_lossy().into_owned()
        })
        .collect();
    let paths = |include_c: bool| {
        let mut paths = vec![
            Value::string("inside.txt"),
            Value::string(&outside[0]),
            Value::string(&outside[1]),
        ];
        if include_c {
            paths.push(Value::string(&outside[2]));
        }
        paths
    };
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![batch("first", paths(false))])),
        (200, completion("First", vec![])),
        (200, completion("", vec![batch("repeat", paths(false))])),
        (200, completion("Repeat", vec![])),
        (200, completion("", vec![batch("wider", paths(true))])),
        (200, completion("Done", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.permissions().set_surface(Surface::Panel);
    let remember = answer_once(&agent, Decision::Remember);
    agent.run_turn("First", &mut |_| Ok(())).unwrap();
    assert!(remember.join().unwrap());
    let grants = agent.permissions().grants();
    assert_eq!(grants.len(), 2, "the inside file has no outside grant");
    assert!(grants.iter().any(|grant| grant.target.ends_with("a.txt")));
    assert!(grants.iter().any(|grant| grant.target.ends_with("b.txt")));
    assert!(result(&agent, "first").get("files").is_some());

    agent.permissions().set_surface(Surface::None);
    agent.run_turn("Repeat", &mut |_| Ok(())).unwrap();
    let repeated = result(&agent, "repeat");
    let files = repeated.get("files").and_then(Value::as_array).unwrap();
    assert_eq!(files.len(), 3);
    assert!(
        named(files, 1, &outside[0])
            .encode()
            .contains("marker-a.txt")
    );
    assert!(
        named(files, 2, &outside[1])
            .encode()
            .contains("marker-b.txt")
    );

    agent.permissions().set_surface(Surface::Panel);
    let deny = answer_once(&agent, Decision::Deny);
    agent.run_turn("Wider", &mut |_| Ok(())).unwrap();
    assert!(deny.join().unwrap(), "a new outside file needs approval");
    let wider = result(&agent, "wider");
    assert_eq!(wider.get("outcome").and_then(Value::as_str), Some("denied"));
    assert!(wider.get("files").is_none());
    assert!(!wider.encode().contains("marker-c.txt"));
    assert_eq!(fixture.finish().len(), 6);
}

#[test]
fn changed_link_target_invalidates_approval_for_a_read_batch() {
    let project = Directory::new();
    let outside = Directory::new();
    let first = outside.path().join("first");
    let second = outside.path().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("data.txt"), "approved marker").unwrap();
    fs::write(second.join("data.txt"), "unapproved marker").unwrap();
    fs::write(project.path().join("inside.txt"), "inside marker").unwrap();
    let link = project.path().join("alias");
    link_directory(&first, &link);
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.permissions().set_surface(Surface::Panel);
    let permissions = agent.permissions();
    let responder = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        let request = loop {
            if let Some((request, _)) = permissions.pending() {
                break request;
            }
            assert!(
                Instant::now() < deadline,
                "an outside batch should request approval"
            );
            thread::sleep(Duration::from_millis(5));
        };
        #[cfg(windows)]
        fs::remove_dir(&link).unwrap();
        #[cfg(unix)]
        fs::remove_file(&link).unwrap();
        link_directory(&second, &link);
        permissions.decide(request.id, Decision::Once).unwrap();
    });
    let call = ToolCall {
        id: "changed-link".into(),
        name: "read".into(),
        arguments: paths(&["inside.txt", "alias/data.txt"]).encode(),
    };
    let gate = agent.permit(&call, &mut |_| Ok(()));
    responder.join().unwrap();
    match gate {
        Gate::Done(result) => {
            assert_eq!(
                result.get("outcome").and_then(Value::as_str),
                Some("unavailable")
            );
            assert!(!result.encode().contains("unapproved marker"));
        }
        Gate::Run { .. } => panic!("approval for first/data.txt must not read second/data.txt"),
    }
}
