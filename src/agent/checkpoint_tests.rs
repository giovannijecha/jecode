//! Checkpoint notices through synthetic provider turns: a change made by
//! another session reaches the next request, survives compaction and resume,
//! and reloads project instructions it restored.
use super::*;
use crate::checkpoints::command;
use crate::checkpoints::session::Session;
use crate::events::Event;
use crate::json;
use crate::test_support::{Directory, HttpFixture, completion as response, tool_call as call};
use std::fs;

struct Fixture {
    home: Directory,
    root: Directory,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            home: Directory::new(),
            root: Directory::new(),
        };
        fs::write(fixture.root.path().join("JECODE.md"), "Use tabs.\n").unwrap();
        fs::write(fixture.root.path().join("a.txt"), "one\n").unwrap();
        fixture
    }

    fn agent(&self, http: &HttpFixture) -> Agent {
        let mut agent = Agent::new(
            OpenRouter::fixture(http.endpoint.clone()),
            Tools::new(self.root.path()).unwrap(),
        );
        agent.enable_sessions(self.home.path()).unwrap();
        agent.use_checkpoints(Session::offline(self.home.path(), agent.tools.root()));
        agent
    }

    /// `jecode checkpoints ...` from another terminal.
    fn cli(&self, words: &[&str]) -> String {
        let words: Vec<String> = words.iter().map(|word| word.to_string()).collect();
        command::offline(self.home.path(), self.root.path(), &words)
            .unwrap()
            .text()
    }
}

fn system(request: &Value) -> String {
    request.get("messages").and_then(Value::as_array).unwrap()[0]
        .get("content")
        .and_then(Value::as_str)
        .unwrap()
        .to_string()
}

/// The point saved before the conversation's last file change.
fn last_before(agent: &Agent) -> String {
    let messages = agent.messages.lock().unwrap();
    let result = messages
        .iter()
        .rev()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .find_map(|message| json::parse(message.get("content")?.as_str()?).ok())
        .unwrap();
    result
        .get("checkpoint")
        .and_then(|checkpoint| checkpoint.get("before"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{result:?}"))
        .to_string()
}

fn notices(agent: &Agent) -> usize {
    agent
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("checkpoint_notice"))
        .count()
}

fn write(path: &str, content: &str) -> Value {
    response(
        "",
        vec![call(
            "write-call",
            "write",
            Value::object([
                ("path", Value::string(path)),
                ("content", Value::string(content)),
            ]),
        )],
    )
}

#[test]
fn a_restore_elsewhere_during_a_turn_reaches_the_next_request_and_reloads_instructions() {
    let fixture = Fixture::new();
    let read = response(
        "",
        vec![call(
            "read-call",
            "read",
            Value::object([("path", Value::string("a.txt"))]),
        )],
    );
    let http = HttpFixture::new(vec![
        (200, write("JECODE.md", "Use spaces.\n")),
        (200, response("Changed the style.", vec![])),
        (200, read),
        (200, response("Noted.", vec![])),
    ]);
    let mut agent = fixture.agent(&http);
    agent.run_turn("Prefer spaces", &mut |_| Ok(())).unwrap();
    let before = last_before(&agent);
    assert_eq!(
        notices(&agent),
        0,
        "the conversation's own changes are not notices"
    );

    agent
        .run_turn("Check a.txt", &mut |event| {
            if matches!(event, Event::ToolFinished { .. }) {
                // The user restores the instructions from another terminal mid-turn.
                fixture.cli(&["restore", &before, "JECODE.md"]);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        fs::read_to_string(fixture.root.path().join("JECODE.md")).unwrap(),
        "Use tabs.\n"
    );
    let requests = http.finish();
    assert_eq!(requests.len(), 4);
    let during = system(&requests[2].body);
    assert!(
        during.contains("Use spaces.") && !during.contains("Checkpoint notices"),
        "{during}"
    );
    let after = system(&requests[3].body);
    assert!(
        after.contains("Use tabs.") && !after.contains("Use spaces."),
        "{after}"
    );
    assert!(after.contains("Checkpoint notices"), "{after}");
    assert!(
        after.contains("by the user (jecode checkpoints): restored"),
        "{after}"
    );
    assert!(
        after.contains("JECODE.md changed; the project instructions were reloaded."),
        "{after}"
    );
    assert_eq!(notices(&agent), 1);
}

#[test]
fn notices_survive_compaction_and_resume_without_being_repeated() {
    let fixture = Fixture::new();
    let rejected = Value::object([(
        "error",
        Value::object([
            ("code", Value::string("context_length_exceeded")),
            ("message", Value::string("Maximum context length exceeded")),
        ]),
    )]);
    let http = HttpFixture::new(vec![
        (200, write("a.txt", "two\n")),
        (200, response("Wrote a.txt.", vec![])),
        (400, rejected),
        (
            200,
            response(
                "Summary: a.txt was written and later restored elsewhere.",
                vec![],
            ),
        ),
        (200, response("Finished.", vec![])),
    ]);
    let mut agent = fixture.agent(&http);
    agent.run_turn("Write a.txt", &mut |_| Ok(())).unwrap();
    let before = last_before(&agent);
    fixture.cli(&["restore", &before, "a.txt"]);
    assert_eq!(
        fs::read_to_string(fixture.root.path().join("a.txt")).unwrap(),
        "one\n"
    );

    // Leave summary room after a rejection of the native prompt and tool contract.
    let prompt = "original objective ".repeat(1500);
    agent.run_turn(&prompt, &mut |_| Ok(())).unwrap();
    let requests = http.finish();
    assert_eq!(requests.len(), 5);
    assert!(agent.context.from > 1, "the turn was compacted");
    let notice = "by the user (jecode checkpoints): restored";
    assert!(system(&requests[2].body).contains(notice));
    assert!(
        system(&requests[4].body).contains(notice),
        "the notice outlives compaction"
    );
    assert_eq!(notices(&agent), 1);
    agent.save_session().unwrap();
    let id = agent.sessions().unwrap().id();
    drop(agent);

    let http = HttpFixture::new(vec![
        (200, response("Resumed.", vec![])),
        (200, response("Seen.", vec![])),
    ]);
    let mut resumed = fixture.agent(&http);
    resumed.resume(&id).unwrap();
    resumed.run_turn("Continue", &mut |_| Ok(())).unwrap();
    assert_eq!(
        notices(&resumed),
        1,
        "a resumed conversation does not repeat notices"
    );
    fixture.cli(&["milestone", "external", "--pin"]);
    fixture.cli(&["config", "--max-file", "4MiB"]);
    resumed.run_turn("Continue again", &mut |_| Ok(())).unwrap();
    assert_eq!(notices(&resumed), 2);
    let requests = http.finish();
    assert!(
        system(&requests[0].body).contains(notice),
        "the projected notice is kept on resume"
    );
    let latest = system(&requests[1].body);
    assert!(
        latest.contains(notice) && latest.contains("limits"),
        "{latest}"
    );
}

#[test]
fn preparing_a_turn_before_resume_does_not_acknowledge_unseen_restores() {
    let fixture = Fixture::new();
    let http = HttpFixture::new(vec![
        (200, write("a.txt", "two\n")),
        (200, response("Wrote a.txt.", vec![])),
    ]);
    let mut agent = fixture.agent(&http);
    agent.run_turn("Write a.txt", &mut |_| Ok(())).unwrap();
    http.finish();
    let before = last_before(&agent);
    fixture.cli(&["restore", &before, "a.txt"]);

    // The worker records a newer workspace point, but no request has seen
    // the restore. Simulate interruption before dispatching that request.
    agent.prepare_turn("Continue after restoring").unwrap();
    agent.checkpoint_turn("Continue after restoring");
    agent.save_session().unwrap();
    let id = agent.sessions().unwrap().id();
    drop(agent);

    let http = HttpFixture::new(vec![(200, response("Resumed.", vec![]))]);
    let mut resumed = fixture.agent(&http);
    resumed.resume(&id).unwrap();
    resumed.run_turn("Continue", &mut |_| Ok(())).unwrap();
    let requests = http.finish();
    assert!(
        system(&requests[0].body).contains("by the user (jecode checkpoints): restored"),
        "a turn point must not acknowledge a notice that never reached a request"
    );
}

#[test]
fn read_only_conversations_keep_their_notice_cursor_on_resume() {
    let fixture = Fixture::new();
    let http = HttpFixture::new(vec![(200, response("Read-only answer.", vec![]))]);
    let mut agent = fixture.agent(&http);
    agent
        .run_turn("Explain the project", &mut |_| Ok(()))
        .unwrap();
    http.finish();
    agent.save_session().unwrap();
    let id = agent.sessions().unwrap().id();
    drop(agent);
    fixture.cli(&["config", "--max-file", "4MiB"]);

    let http = HttpFixture::new(vec![(200, response("Resumed.", vec![]))]);
    let mut resumed = fixture.agent(&http);
    resumed.resume(&id).unwrap();
    resumed.run_turn("Continue", &mut |_| Ok(())).unwrap();
    let requests = http.finish();
    assert!(
        system(&requests[0].body).contains("limits"),
        "a conversation that never changed files still needs workspace notices"
    );
}

#[test]
fn restores_during_request_preparation_reach_that_request() {
    let fixture = Fixture::new();
    let http = HttpFixture::new(vec![
        (200, write("JECODE.md", "Use spaces.\n")),
        (200, response("Changed instructions.", vec![])),
        (200, response("Seen restore.", vec![])),
    ]);
    let mut agent = fixture.agent(&http);
    agent
        .run_turn("Change instructions", &mut |_| Ok(()))
        .unwrap();
    let before = last_before(&agent);
    let mut restored = false;
    agent
        .run_turn("Continue", &mut |event| {
            if matches!(event, Event::Waiting { .. }) && !restored {
                fixture.cli(&["restore", &before, "JECODE.md"]);
                restored = true;
            }
            Ok(())
        })
        .unwrap();
    assert!(restored);
    let requests = http.finish();
    let latest = system(&requests[2].body);
    assert!(latest.contains("Use tabs."), "{latest}");
    assert!(!latest.contains("Use spaces."), "{latest}");
    assert!(latest.contains("Checkpoint notices"), "{latest}");
}

#[test]
fn large_scope_reaches_the_model_with_full_baseline_and_exact_before_refs_restore() {
    let fixture = Fixture::new();
    let http = HttpFixture::new(vec![
        (200, write("a.txt", "changed\n")),
        (200, response("Saved.", vec![])),
        (200, response("Restored.", vec![])),
    ]);
    let mut agent = fixture.agent(&http);
    let workspace = agent
        .tools
        .checkpoints()
        .unwrap()
        .workspace()
        .unwrap()
        .clone();
    workspace
        .configure(Some(1 << 20), Some(512 << 10), "user")
        .unwrap();
    for number in 0..4 {
        fs::write(
            fixture.root.path().join(format!("bulk-{number}.bin")),
            vec![number; 384 << 10],
        )
        .unwrap();
    }
    let verified = workspace.verify().unwrap();
    assert!(verified.limited.is_none());
    assert!(workspace.selective().is_none());
    agent.run_turn("Change a.txt", &mut |_| Ok(())).unwrap();
    let before = last_before(&agent);
    fixture.cli(&["restore", &before, "a.txt"]);
    assert_eq!(
        fs::read(fixture.root.path().join("a.txt")).unwrap(),
        b"one\n"
    );
    agent.run_turn("Continue", &mut |_| Ok(())).unwrap();
    let requests = http.finish();
    for request in &requests {
        let context = system(&request.body);
        assert!(!context.contains("selective protection"), "{context}");
        let tools = request.body.get("tools").unwrap().encode();
        assert!(tools.contains("full in-scope baseline"));
        assert!(tools.contains("checkpoint.before"));
    }
    assert!(system(&requests[2].body).contains("Checkpoint notices"));
}
