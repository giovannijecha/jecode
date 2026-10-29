use super::permission::Gate;
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    permissions::{Decision, Level, Surface},
    sessions::Stage,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::{fs, thread};

fn read_call(id: &str, path: &std::path::Path) -> Value {
    tool_call(
        id,
        "read",
        Value::object([("path", Value::string(path.to_string_lossy()))]),
    )
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
fn workspace_shell_is_confined_even_when_parallel_and_full_runs_outside() {
    let project = Directory::new();
    let outside = Directory::new();
    let marker = outside.path().join("full-bash.txt");
    let path = marker.to_string_lossy().into_owned();
    let bash = Value::object([
        (
            "command",
            Value::string(format!("printf FULL_BASH_OK > '{path}'")),
        ),
        ("parallel", Value::Bool(true)),
    ]);
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion("", vec![tool_call("workspace-bash", "bash", bash.clone())]),
        ),
        (200, completion("Workspace refused", vec![])),
        (
            200,
            completion("", vec![tool_call("full-bash", "bash", bash)]),
        ),
        (200, completion("Full executed", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.run_turn("Try Bash", &mut |_| Ok(())).unwrap();
    let refused = result(&agent, "workspace-bash");
    if cfg!(windows) {
        assert_eq!(
            refused.get("exit_code"),
            Some(&Value::number(1)),
            "{refused:?}"
        );
    } else {
        assert!(refused.encode().contains("only on Windows"), "{refused:?}");
    }
    assert!(!marker.exists());

    agent.permissions().set_level(Level::Full).unwrap();
    agent.run_turn("Run Bash at Full", &mut |_| Ok(())).unwrap();
    let executed = result(&agent, "full-bash");
    assert_eq!(
        executed.get("exit_code"),
        Some(&Value::number(0)),
        "{executed:?}"
    );
    assert_eq!(fs::read_to_string(marker).unwrap(), "FULL_BASH_OK");
    assert_eq!(fixture.finish().len(), 4);
}

#[test]
fn browser_control_requires_full_while_read_only_web_is_available() {
    let project = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(project.path()).unwrap(),
    );
    let call = |id: &str, name: &str, arguments: Value| {
        crate::backend::ToolCall::parse(&tool_call(id, name, arguments)).unwrap()
    };
    let inspect = call(
        "inspect",
        "browser",
        Value::object([
            ("action", Value::string("navigate")),
            ("url", Value::string("https://example.org")),
        ]),
    );
    let click = call(
        "click",
        "browser",
        Value::object([("action", Value::string("click"))]),
    );
    let raw = call(
        "raw",
        "browser",
        Value::object([
            ("action", Value::string("cdp")),
            ("method", Value::string("Page.navigate")),
            (
                "params",
                Value::object([("url", Value::string("file:///outside"))]),
            ),
        ]),
    );
    let post = call(
        "post",
        "web",
        Value::object([
            ("action", Value::string("fetch")),
            ("url", Value::string("https://example.org")),
            ("method", Value::string("POST")),
        ]),
    );
    let get = call(
        "get",
        "web",
        Value::object([
            ("action", Value::string("fetch")),
            ("url", Value::string("https://example.org")),
            ("method", Value::string("GET")),
        ]),
    );
    let close = call(
        "close",
        "browser",
        Value::object([("action", Value::string("close"))]),
    );
    let stop = call(
        "stop",
        "browser",
        Value::object([("action", Value::string("stop"))]),
    );
    let verdict = |call: &crate::backend::ToolCall| match agent.permit(call, &mut |_| Ok(())) {
        Gate::Run { .. } => "run".to_string(),
        Gate::Done(value) => value
            .get("outcome")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
    };
    agent.permissions().set_level(Level::Read).unwrap();
    assert_eq!(verdict(&get), "run");
    assert_eq!(verdict(&close), "run");
    assert_eq!(verdict(&stop), "run");
    for denied in [&inspect, &click, &raw, &post] {
        assert_eq!(verdict(denied), "unavailable");
    }
    agent.permissions().set_level(Level::Workspace).unwrap();
    assert_eq!(verdict(&post), "run");
    assert_eq!(verdict(&close), "run");
    assert_eq!(verdict(&stop), "run");
    assert_eq!(verdict(&inspect), "unavailable");
    assert_eq!(verdict(&click), "unavailable");
    assert_eq!(verdict(&raw), "unavailable");
    agent.permissions().set_level(Level::Full).unwrap();
    assert_eq!(verdict(&inspect), "run");
    assert_eq!(verdict(&click), "run");
    assert_eq!(verdict(&raw), "run");
}

/// Decides every request the way the user would from the TUI.
fn answer_requests(agent: &Agent, decision: Decision) -> thread::JoinHandle<usize> {
    let permissions = agent.permissions();
    thread::spawn(move || {
        let mut decided = 0;
        for _ in 0..400 {
            if let Some((request, _)) = permissions.pending() {
                permissions.decide(request.id, decision).unwrap();
                decided += 1;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        decided
    })
}

#[test]
fn workspace_asks_outside_and_a_remembered_grant_survives_resume() {
    let project = Directory::new();
    let other = Directory::new();
    let home = Directory::new();
    let outside = other.path().join("session.rs");
    fs::write(&outside, "wall-clock times\n").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![read_call("read-1", &outside)])),
        (200, completion("Read it", vec![])),
        (200, completion("", vec![read_call("read-2", &outside)])),
        (200, completion("Again, without asking", vec![])),
    ]);
    let make_agent = || {
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(project.path()).unwrap(),
        )
    };
    let mut agent = make_agent();
    agent.enable_sessions(home.path()).unwrap();
    assert_eq!(agent.permissions().level(), Level::Workspace);
    agent.permissions().set_surface(Surface::Panel);
    let user = answer_requests(&agent, Decision::Remember);
    let mut notes = Vec::new();
    agent
        .run_turn("Compare webterminal", &mut |event| {
            if let Event::ToolNote { text, .. } = event {
                notes.push(text);
            }
            Ok(())
        })
        .unwrap();
    assert!(result(&agent, "read-1").encode().contains("wall-clock"));
    assert_eq!(notes, ["Approved for this conversation"]);
    let id = agent.sessions().unwrap().id();
    agent.save_session().unwrap();
    drop(agent);
    assert_eq!(user.join().unwrap(), 1);

    let mut resumed = make_agent();
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.permissions().grants().len(), 1);
    // No surface: the grant alone lets the same read run.
    resumed.run_turn("Read it again", &mut |_| Ok(())).unwrap();
    assert!(result(&resumed, "read-2").encode().contains("wall-clock"));
    assert_eq!(fixture.finish().len(), 4);
}

#[test]
fn deny_read_level_and_headless_requests_run_nothing() {
    let project = Directory::new();
    let other = Directory::new();
    let outside = other.path().join("session.rs");
    fs::write(&outside, "secret plan\n").unwrap();
    let bash = tool_call(
        "bash-1",
        "bash",
        Value::object([("command", Value::string("echo ran > ran.txt"))]),
    );
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![read_call("denied", &outside)])),
        (200, completion("Understood", vec![])),
        (200, completion("", vec![read_call("headless", &outside)])),
        (200, completion("Cannot ask", vec![])),
        (200, completion("", vec![bash])),
        (200, completion("Read only", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.permissions().set_surface(Surface::Panel);
    let user = answer_requests(&agent, Decision::Deny);
    agent.run_turn("Compare", &mut |_| Ok(())).unwrap();
    let denied = result(&agent, "denied");
    assert_eq!(
        denied.get("outcome").and_then(Value::as_str),
        Some("denied")
    );
    assert!(!denied.encode().contains("secret plan"));
    user.join().unwrap();

    agent.permissions().set_surface(Surface::None);
    agent.run_turn("Compare again", &mut |_| Ok(())).unwrap();
    let headless = result(&agent, "headless");
    assert_eq!(
        headless.get("outcome").and_then(Value::as_str),
        Some("unavailable")
    );

    agent.permissions().set_level(Level::Read).unwrap();
    agent.run_turn("Write a marker", &mut |_| Ok(())).unwrap();
    let refused = result(&agent, "bash-1");
    assert!(refused.encode().contains("Read permission level"));
    assert!(!project.path().join("ran.txt").exists());
    assert_eq!(fixture.finish().len(), 6);
}

#[test]
fn collaborators_share_the_level_grants_and_ask_under_their_own_id() {
    let project = Directory::new();
    let parent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(project.path()).unwrap(),
    );
    let child = parent.permissions().for_agent("a1");
    parent.permissions().set_level(Level::Full).unwrap();
    assert_eq!(child.level(), Level::Full);
    parent.permissions().set_level(Level::Workspace).unwrap();
    parent.permissions().set_surface(Surface::Panel);
    let access = crate::permissions::Access {
        kind: crate::permissions::Kind::Edits,
        target: "elsewhere".into(),
        place: None,
        outside: true,
        base: None,
        identity: None,
    };
    let request = child.begin("child-call", "write", &access).unwrap();
    assert_eq!(request.agent.as_deref(), Some("a1"));
    parent
        .permissions()
        .decide(request.id, Decision::Remember)
        .unwrap();
    assert!(matches!(
        parent.permissions().check(&access),
        crate::permissions::Verdict::Allow
    ));
}

#[test]
fn a_request_pending_at_quit_is_recorded_as_not_run() {
    let project = Directory::new();
    let other = Directory::new();
    let home = Directory::new();
    let outside = other.path().join("session.rs");
    fs::write(&outside, "x").unwrap();
    let make_agent = || {
        Agent::new(
            OpenRouter::fixture("http://127.0.0.1:9".into()),
            Tools::new(project.path()).unwrap(),
        )
    };
    let mut agent = make_agent();
    agent.enable_sessions(home.path()).unwrap();
    agent.prepare_turn("Compare").unwrap();
    let call = read_call("read-1", &outside);
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::Null),
        ("tool_calls", Value::Array(vec![call])),
    ]));
    agent.checkpoint(Stage::Completion).unwrap();
    agent.checkpoint(Stage::Tool("read-1".into())).unwrap();
    agent.permissions().set_surface(Surface::Panel);
    let arguments = Value::object([("path", Value::string(outside.to_string_lossy()))]);
    let access = agent.tools.access("read", &arguments.encode()).unwrap();
    agent
        .permissions()
        .begin("read-1", "read", &access)
        .unwrap();
    let id = agent.sessions().unwrap().id();
    drop(agent);

    let mut resumed = make_agent();
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    let recorded = result(&resumed, "read-1");
    assert_eq!(
        recorded.get("outcome").and_then(Value::as_str),
        Some("not_executed")
    );
    assert!(recorded.encode().contains("permission decision"));
}

#[test]
fn a_waiting_approval_does_not_run_after_switching_to_read() {
    let project = Directory::new();
    let outside = Directory::new();
    let target = outside.path().join("sample.txt");
    fs::write(&target, "outside content").unwrap();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.permissions().set_surface(Surface::Panel);
    let permissions = agent.permissions();
    let responder = thread::spawn(move || {
        let request = loop {
            if let Some((request, _)) = permissions.pending() {
                break request;
            }
            thread::yield_now();
        };
        permissions.set_level(Level::Read).unwrap();
        permissions.decide(request.id, Decision::Once).unwrap();
    });
    let call = crate::backend::ToolCall::parse(&read_call("read-1", &target)).unwrap();
    let gate = agent.permit(&call, &mut |_| Ok(()));
    responder.join().unwrap();
    match gate {
        Gate::Done(result) => assert_eq!(
            result.get("outcome").and_then(Value::as_str),
            Some("unavailable")
        ),
        Gate::Run { .. } => panic!("Read must not run a waiting outside call"),
    }
}

#[test]
fn collaborator_request_and_answer_are_recorded_in_its_own_session() {
    let project = Directory::new();
    let parent_home = Directory::new();
    let child_home = Directory::new();
    let outside = Directory::new();
    let mut parent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(parent_home.path()).unwrap();
    parent.permissions().set_surface(Surface::Panel);
    let mut child = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(project.path()).unwrap(),
    );
    child.child = true;
    child.permissions = parent.permissions().for_agent("a1");
    child.enable_sessions(child_home.path()).unwrap();
    let arguments = Value::object([(
        "path",
        Value::string(outside.path().join("file.txt").to_string_lossy()),
    )]);
    let access = child.tools.access("read", &arguments.encode()).unwrap();
    let main_request = parent
        .permissions()
        .begin("child-read", "read", &access)
        .unwrap();
    let request = child
        .permissions()
        .begin("child-read", "read", &access)
        .unwrap();
    let child_events = child.sessions().unwrap().snapshot().events;
    assert!(crate::permissions::pending_request(
        &child_events,
        "child-read"
    ));
    let parent_events = parent.sessions().unwrap().snapshot().events;
    assert!(crate::permissions::pending_request(
        &parent_events,
        "child-read"
    ));

    parent
        .permissions()
        .decide(request.id, Decision::Once)
        .unwrap();
    child
        .permissions()
        .record_agent_decision(&request, Decision::Once)
        .unwrap();
    let child_events = child.sessions().unwrap().snapshot().events;
    assert!(!crate::permissions::pending_request(
        &child_events,
        "child-read"
    ));
    let parent_events = parent.sessions().unwrap().snapshot().events;
    assert!(crate::permissions::pending_request(
        &parent_events,
        "child-read"
    ));
    parent
        .permissions()
        .decide(main_request.id, Decision::Deny)
        .unwrap();
}

#[test]
fn failed_permission_event_abandons_the_request_without_running() {
    let project = Directory::new();
    let outside = Directory::new();
    let target = outside.path().join("sample.txt");
    fs::write(&target, "outside content").unwrap();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.permissions().set_surface(Surface::Panel);
    let call = crate::backend::ToolCall::parse(&read_call("read-1", &target)).unwrap();
    let gate = agent.permit(&call, &mut |_| Err("display closed".into()));
    assert!(agent.permissions().pending().is_none());
    match gate {
        Gate::Done(result) => assert_eq!(
            result.get("outcome").and_then(Value::as_str),
            Some("not_executed")
        ),
        Gate::Run { .. } => panic!("a request that was not shown must not run"),
    }
}

#[cfg(any(windows, unix))]
fn make_symlink(source: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    return std::os::windows::fs::symlink_file(source, link);
    #[cfg(unix)]
    return std::os::unix::fs::symlink(source, link);
}

#[cfg(any(windows, unix))]
#[test]
fn approved_symlink_cannot_switch_to_a_different_outside_file() {
    let project = Directory::new();
    let outside = Directory::new();
    let first = outside.path().join("first.txt");
    let second = outside.path().join("second.txt");
    fs::write(&first, "first").unwrap();
    fs::write(&second, "second").unwrap();
    let link = project.path().join("link.txt");
    if make_symlink(&first, &link).is_err() {
        return; // Windows may require Developer Mode for symlinks.
    }
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.permissions().set_surface(Surface::Panel);
    let permissions = agent.permissions();
    let responder = thread::spawn(move || {
        let request = loop {
            if let Some((request, _)) = permissions.pending() {
                break request;
            }
            thread::yield_now();
        };
        fs::remove_file(&link).unwrap();
        make_symlink(&second, &link).unwrap();
        permissions.decide(request.id, Decision::Once).unwrap();
    });
    let call =
        crate::backend::ToolCall::parse(&read_call("read-1", &project.path().join("link.txt")))
            .unwrap();
    let gate = agent.permit(&call, &mut |_| Ok(()));
    responder.join().unwrap();
    match gate {
        Gate::Done(result) => assert_eq!(
            result.get("outcome").and_then(Value::as_str),
            Some("unavailable")
        ),
        Gate::Run { .. } => panic!("approval for first.txt must not read second.txt"),
    }
}
