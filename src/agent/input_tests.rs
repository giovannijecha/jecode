use super::*;
use crate::{
    events::Event,
    inbox::Inbox,
    sessions::Input,
    test_support::{Directory, HttpFixture, Request, Response, completion, tool_call},
};
use std::sync::mpsc;

const STOP: &str =
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

fn agent(directory: &Directory, fixture: &HttpFixture) -> Agent {
    Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    )
}

fn content(text: &str) -> String {
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}},\"finish_reason\":null}}]}}\n\n"
    )
}

/// Submits prompts as the interface does: queued, then recorded in its input.
fn send(inbox: &Inbox, prompts: &[&str]) {
    inbox.update(|entries| {
        for prompt in prompts {
            entries.push((*prompt).into()).unwrap();
        }
        entries.record();
    });
}

fn messages(request: &Request) -> Vec<(String, String)> {
    let text = |message: &Value, key| {
        message
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    request
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| text(message, "role") != "system")
        .map(|message| (text(message, "role"), text(message, "content")))
        .collect()
}

fn users(request: &Request) -> Vec<String> {
    messages(request)
        .into_iter()
        .filter(|(role, _)| role == "user")
        .map(|(_, content)| content)
        .collect()
}

/// Messages, accepted input and tool events in the order they were emitted.
fn describe(event: &Event) -> Option<String> {
    match event {
        Event::Message { text } => Some(format!("message {text}")),
        Event::Accepted { prompts } => Some(format!(
            "accepted {}",
            prompts
                .iter()
                .map(|prompt| prompt.text.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Event::ToolFinished { id, .. } => Some(format!("tool {id}")),
        Event::InputHeld => Some("held".into()),
        _ => None,
    }
}

#[test]
fn messages_sent_while_a_response_streams_join_the_next_request_once_in_order() {
    let directory = Directory::new();
    let (release, gate) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        Response::GatedStream {
            head: content("First."),
            tail: STOP.into(),
            release: gate,
        },
        Response::Json(200, completion("Second.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    let inbox = agent.inbox();
    let mut log = Vec::new();
    agent
        .run_turn("start", &mut |event| {
            if matches!(event, Event::Streaming { .. }) && log.is_empty() {
                // The response is mid-stream; it completes before the messages join.
                send(&inbox, &["one", "/model fixture/other", "two"]);
                release.send(()).unwrap();
            }
            log.extend(describe(&event));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        log,
        ["message First.", "accepted one, two", "message Second."]
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(users(&requests[0]), ["start"]);
    assert_eq!(
        messages(&requests[1]),
        [
            ("user".into(), "start".into()),
            ("assistant".into(), "First.".into()),
            ("user".into(), "one".into()),
            ("user".into(), "two".into()),
        ]
    );
    inbox.with(|entries| {
        assert_eq!(entries.unacknowledged(), ["one", "two"]);
        assert_eq!(entries.pop_front().unwrap(), "/model fixture/other");
    });
}

#[test]
fn input_during_a_tool_batch_waits_for_every_result_and_later_responses_continue() {
    let directory = Directory::new();
    let write = |id: &str| {
        tool_call(
            id,
            "write",
            Value::object([
                ("path", Value::string(format!("{id}.txt"))),
                ("content", Value::string(id)),
            ]),
        )
    };
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![write("a"), write("b")])),
        (200, completion("Second.", vec![])),
        (200, completion("Third.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    let inbox = agent.inbox();
    let mut log = Vec::new();
    agent
        .run_turn("start", &mut |event| {
            match &event {
                Event::ToolStarted { id, .. } if id == "a" => send(&inbox, &["during tools"]),
                Event::Message { text } if text == "Second." => send(&inbox, &["at the end"]),
                _ => {}
            }
            log.extend(describe(&event));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        log,
        [
            "tool a",
            "tool b",
            "accepted during tools",
            "message Second.",
            "accepted at the end",
            "message Third.",
        ]
    );
    assert!(directory.path().join("a.txt").exists());
    assert!(directory.path().join("b.txt").exists());
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    let roles: Vec<_> = messages(&requests[1])
        .into_iter()
        .map(|(role, _)| role)
        .collect();
    assert_eq!(roles, ["user", "assistant", "tool", "tool", "user"]);
    assert_eq!(users(&requests[2]), ["start", "during tools", "at the end"]);
}

#[test]
fn an_edit_keeps_the_turn_open_until_saved_and_holds_later_messages_behind_it() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("First.", vec![])),
        (200, completion("Second.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    let inbox = agent.inbox();
    send(&inbox, &["edited", "after"]);
    let held = inbox.with(|entries| entries.prompts().next().unwrap().0);
    inbox.update(|entries| entries.hold(held).unwrap());
    let mut log = Vec::new();
    agent
        .run_turn("start", &mut |event| {
            if matches!(event, Event::InputHeld) {
                inbox.update(|entries| entries.release(Some("edited and saved".into())));
            }
            log.extend(describe(&event));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        log,
        [
            "message First.",
            "held",
            "accepted edited and saved, after",
            "message Second.",
        ]
    );
    let requests = fixture.finish();
    assert_eq!(users(&requests[0]), ["start"]);
    assert_eq!(users(&requests[1]), ["start", "edited and saved", "after"]);
}

#[test]
fn stopping_while_an_edit_holds_the_turn_keeps_its_messages_pending() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("First.", vec![]))]);
    let mut agent = agent(&directory, &fixture);
    let inbox = agent.inbox();
    let cancellation = agent.cancellation();
    send(&inbox, &["edited"]);
    let held = inbox.with(|entries| entries.prompts().next().unwrap().0);
    inbox.update(|entries| entries.hold(held).unwrap());
    let error = agent
        .run_turn("start", &mut |event| {
            if matches!(event, Event::InputHeld) {
                cancellation.cancel();
                inbox.wake();
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error, "Operation cancelled");
    assert_eq!(inbox.with(|entries| entries.len()), 1);
    assert!(
        !agent
            .archive()
            .document()
            .encode()
            .contains("\"content\":\"edited\"")
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn a_retry_is_rebuilt_with_messages_sent_while_it_waited() {
    let directory = Directory::new();
    let unavailable = Value::object([(
        "error",
        Value::object([
            ("code", Value::number(503)),
            ("message", Value::string("Provider unavailable")),
        ]),
    )]);
    let fixture = HttpFixture::new(vec![
        (503, unavailable),
        (200, completion("Recovered.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    let inbox = agent.inbox();
    agent
        .run_turn("start", &mut |event| {
            if matches!(event, Event::Recovering { .. }) {
                send(&inbox, &["while retrying"]);
            }
            Ok(())
        })
        .unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(users(&requests[0]), ["start"]);
    assert_eq!(users(&requests[1]), ["start", "while retrying"]);
}

#[test]
fn messages_sent_during_compaction_join_the_request_after_it() {
    let directory = Directory::new();
    let prompt = "original objective ".repeat(750);
    let fixture = HttpFixture::new(vec![
        (
            400,
            Value::object([(
                "error",
                Value::object([
                    ("code", Value::string("context_length_exceeded")),
                    ("message", Value::string("Maximum context length exceeded")),
                ]),
            )]),
        ),
        (200, completion("Original objective retained.", vec![])),
        (200, completion("Finished.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    let inbox = agent.inbox();
    let mut sent = false;
    agent
        .run_turn(prompt.as_str(), &mut |event| {
            if matches!(event, Event::Maintenance { .. }) && !sent {
                send(&inbox, &["during compaction"]);
                sent = true;
            }
            Ok(())
        })
        .unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(!requests[1].body.encode().contains("during compaction"));
    assert_eq!(
        users(&requests[2]).last().map(String::as_str),
        Some("during compaction")
    );
}

fn saved_agent(home: &Directory, directory: &Directory, fixture: &HttpFixture) -> Agent {
    let mut agent = agent(directory, fixture);
    agent.enable_sessions(home.path()).unwrap();
    agent
}

#[test]
fn a_failed_save_keeps_accepted_input_pending_and_out_of_the_conversation() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("First.", vec![]))]);
    let mut agent = saved_agent(&home, &directory, &fixture);
    agent.prepare_turn("start").unwrap();
    let handle = agent.sessions().unwrap();
    let bucket = std::fs::read_dir(home.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = bucket.join(format!("{}.jsonl", handle.id()));
    let original = std::fs::metadata(&path).unwrap().permissions();
    let inbox = agent.inbox();
    let result = agent.run_turn("start", &mut |event| {
        if matches!(event, Event::Message { .. }) {
            send(&inbox, &["late"]);
            let mut permissions = original.clone();
            permissions.set_readonly(true);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
        Ok(())
    });
    assert!(result.is_err());
    std::fs::set_permissions(&path, original).unwrap();
    assert_eq!(inbox.with(|entries| entries.len()), 1);
    assert!(inbox.with(|entries| entries.unacknowledged().is_empty()));
    assert!(!agent.archive().document().encode().contains("late"));
    agent.save_session().unwrap();
    assert!(!handle.snapshot().value().encode().contains("late"));
    fixture.finish();
}

#[test]
fn accepted_input_is_saved_once_even_when_the_interface_never_shows_it() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("First.", vec![]))]);
    let mut agent = saved_agent(&home, &directory, &fixture);
    let inbox = agent.inbox();
    let handle = agent.sessions().unwrap();
    let error = agent
        .run_turn("start", &mut |event| match event {
            Event::Message { .. } => {
                send(&inbox, &["late"]);
                handle.input(Input {
                    queued: vec!["late".into()],
                    ..Input::default()
                });
                Ok(())
            }
            Event::Accepted { .. } => Err("Terminal output closed".into()),
            _ => Ok(()),
        })
        .unwrap_err();
    assert_eq!(error, "Terminal output closed");
    assert_eq!(inbox.with(|entries| entries.unacknowledged()), ["late"]);
    let id = handle.id();
    drop(handle);
    drop(agent);
    let unused = HttpFixture::new(vec![]);
    let mut resumed = saved_agent(&home, &directory, &unused);
    resumed.resume(&id).unwrap();
    let document = resumed.sessions().unwrap().snapshot();
    assert!(document.input.queued.is_empty());
    assert!(document.input.paused.is_empty());
    let late = document
        .messages
        .iter()
        .filter(|message| message.get("content").and_then(Value::as_str) == Some("late"))
        .count();
    assert_eq!(late, 1);
    fixture.finish();
    unused.finish();
}

#[test]
fn plain_output_ignores_input_notifications_and_a_turn_without_input_closes() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Plain answer.", vec![]))]);
    let mut agent = agent(&directory, &fixture);
    let (mut output, mut status) = (Vec::new(), Vec::new());
    let mut console = crate::console::Console::new(&mut output, &mut status);
    use crate::events::EventSink;
    console
        .emit(Event::Accepted {
            prompts: vec!["not shown".into()],
        })
        .unwrap();
    console.emit(Event::InputHeld).unwrap();
    agent.run_turn("question", &mut console).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Plain answer."));
    assert!(!output.contains("not shown"));
    assert!(!String::from_utf8(status).unwrap().contains("not shown"));
    assert_eq!(fixture.finish().len(), 1);
}
