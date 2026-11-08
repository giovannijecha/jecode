use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::context::digest::ANCHOR;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use crate::tools::Tools;

const WINDOW: Limits = Limits {
    context: 500_000,
    output: Some(4096),
};

fn setup(responses: Vec<(u16, Value)>) -> (Agent, HttpFixture, Directory) {
    let directory = Directory::new();
    let fixture = HttpFixture::new(responses);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.client.fixture_limits(WINDOW.context, WINDOW.output);
    (agent, fixture, directory)
}

fn user(text: &str) -> Value {
    Value::object([
        ("role", Value::string("user")),
        ("content", Value::string(text)),
    ])
}

fn answer(text: &str) -> Value {
    Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string(text)),
    ])
}

fn read_exchange(id: &str, path: &str, payload: &str) -> [Value; 2] {
    [
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![tool_call(
                    id,
                    "read",
                    Value::object([("path", Value::string(path))]),
                )]),
            ),
        ]),
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string(id)),
            (
                "content",
                Value::string(
                    Value::object([
                        ("content", Value::string(payload)),
                        ("lines_returned", Value::number(10)),
                        ("total_lines", Value::number(10)),
                    ])
                    .encode(),
                ),
            ),
        ]),
    ]
}

fn compact(agent: &mut Agent) {
    agent
        .compact_if_needed(Some(WINDOW), false, &mut |_| Ok(()))
        .unwrap();
}

fn encoded(agent: &Agent) -> String {
    agent
        .transport_messages()
        .iter()
        .map(Value::encode)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_new_turn_keeps_exact_records_and_needs_no_model_call_for_them() {
    let (mut agent, fixture, _directory) = setup(vec![
        (200, completion("First answer.", vec![])),
        (200, completion("Second answer.", vec![])),
    ]);
    agent
        .run_turn("Every behavior change gets a CHANGELOG line.", &mut |_| {
            Ok(())
        })
        .unwrap();
    {
        let mut messages = agent.messages.lock().unwrap();
        messages.extend(read_exchange("big", "src/big.js", &"x".repeat(240_000)));
        messages.push(answer("Root cause: the tokenizer split quotes."));
    }
    let original = agent.messages.lock().unwrap().clone();
    agent.run_turn("Second request", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2, "records take no model request");
    let body = requests[1].body.encode();
    assert!(body.contains("Native record of this conversation"));
    assert!(body.contains("Every behavior change gets a CHANGELOG line."));
    assert!(body.contains("Root cause: the tokenizer split quotes."));
    assert!(body.contains("read src/big.js (10 lines)"));
    let boundary = body.find("End of record.").expect("records close");
    assert!(body.find("Root cause: the tokenizer").unwrap() < boundary);
    assert!(
        boundary < body.rfind("Second request").unwrap(),
        "the new request reads as live, not as the record's last line"
    );
    assert!(!body.contains("xxxxxxxx"), "payloads stay in history");
    assert!(
        !body.contains(ANCHOR),
        "the marker never reaches a provider"
    );
    assert_eq!(agent.context.digests.len(), 1);
    assert_eq!(
        agent.messages.lock().unwrap()[..original.len()],
        original[..],
        "history is untouched"
    );
    let retrieved = agent.execute_tool(&crate::backend::openrouter::ToolCall {
        id: "original".into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string("history:4"))]).encode(),
    });
    assert!(retrieved.encode().contains("xxxxxxxx"));
}

#[test]
fn a_message_sent_while_tools_ran_never_hides_their_unread_results() {
    let (mut agent, fixture, _directory) = setup(vec![]);
    agent.prepare_turn("First").unwrap();
    {
        let mut messages = agent.messages.lock().unwrap();
        for index in 0..6 {
            messages.extend(read_exchange(
                &format!("old{index}"),
                &format!("src/old{index}.js"),
                &"o".repeat(120_000),
            ));
        }
        let unread = format!("UNREAD-RESULT {}", "n".repeat(240_000));
        messages.extend(read_exchange("new", "src/new.js", &unread));
        messages.push(user("Also check the docs"));
    }
    compact(&mut agent);
    assert_eq!(agent.context.digests.len(), 1, "work under way is reduced");
    let view = encoded(&agent);
    assert!(view.contains(&format!("UNREAD-RESULT {}", "n".repeat(240_000))));
    assert!(view.contains("Also check the docs"));
    assert!(!view.contains("oooooooo"));
    assert!(fixture.finish().is_empty());
}

#[test]
fn an_oversized_latest_exchange_under_the_budget_stays_whole() {
    let (mut agent, fixture, _directory) = setup(vec![]);
    agent.prepare_turn("First").unwrap();
    let unread = format!("UNREAD-RESULT {}", "n".repeat(600_000));
    agent
        .messages
        .lock()
        .unwrap()
        .extend(read_exchange("new", "src/new.js", &unread));
    compact(&mut agent);
    assert!(
        encoded(&agent).contains(&unread),
        "no preview below the budget"
    );
    assert!(fixture.finish().is_empty());
}

#[test]
fn at_the_budget_records_that_fit_beat_a_paid_summary() {
    let (mut agent, fixture, _directory) = setup(vec![]);
    agent.prepare_turn("First").unwrap();
    {
        let mut messages = agent.messages.lock().unwrap();
        messages.extend(read_exchange("old", "src/old.js", &"o".repeat(16_000)));
        messages.extend(read_exchange("new", "src/new.js", &"n".repeat(80_000)));
    }
    // A budget just below the estimate, where freeing little is still worthwhile.
    let environment = agent.environment();
    let estimate = {
        let messages = agent.messages.lock().unwrap();
        agent.context_estimate(&environment, &messages)
    };
    let limits = Limits {
        context: (estimate - 100) * 100 / agent.auto_compact_percent() as usize + 1,
        output: Some(4096),
    };
    agent
        .compact_if_needed(Some(limits), false, &mut |_| Ok(()))
        .unwrap();
    assert_eq!(agent.context.digests.len(), 1);
    assert!(encoded(&agent).contains("nnnnnnnn"));
    assert!(fixture.finish().is_empty(), "no summary request");
}

#[test]
fn a_small_new_turn_changes_nothing() {
    let (mut agent, fixture, _directory) = setup(vec![]);
    agent.prepare_turn("First").unwrap();
    agent
        .messages
        .lock()
        .unwrap()
        .extend(read_exchange("small", "a.js", &"x".repeat(4_000)));
    {
        let mut messages = agent.messages.lock().unwrap();
        messages.push(answer("Done."));
        messages.push(user("Next"));
        assert!(
            agent.turn_start(&messages).is_some(),
            "this is a new turn, so only the gain rule can keep it"
        );
    }
    compact(&mut agent);
    assert!(agent.context.digests.is_empty());
    assert_eq!(agent.context.from, 1);
    assert!(fixture.finish().is_empty());
}

#[test]
fn work_under_way_keeps_its_latest_exchange_even_when_oversized() {
    let (mut agent, fixture, _directory) = setup(vec![]);
    agent.prepare_turn("Keep going").unwrap();
    {
        let mut messages = agent.messages.lock().unwrap();
        for index in 0..6 {
            messages.extend(read_exchange(
                &format!("old{index}"),
                &format!("src/old{index}.js"),
                &"o".repeat(120_000),
            ));
        }
        let unread = format!("UNREAD-RESULT {}", "n".repeat(200_000));
        messages.extend(read_exchange("new", "src/new.js", &unread));
    }
    compact(&mut agent);
    assert_eq!(agent.context.digests.len(), 1);
    let messages = agent.messages.lock().unwrap().clone();
    assert_eq!(
        messages[agent.context.from]
            .get("role")
            .and_then(Value::as_str),
        Some("assistant"),
        "a call and its results stay on the same side"
    );
    let request = encoded(&agent);
    assert!(request.contains("UNREAD-RESULT"));
    assert!(request.contains("read src/old0.js (10 lines)"));
    assert!(!request.contains("oooooooo"));
    assert!(fixture.finish().is_empty());
}

#[test]
fn later_digests_only_append_so_the_cached_prefix_survives() {
    let (mut agent, _fixture, _directory) = setup(vec![]);
    agent.prepare_turn("First").unwrap();
    {
        let mut messages = agent.messages.lock().unwrap();
        messages.extend(read_exchange("a", "a.js", &"x".repeat(200_000)));
        messages.push(answer("Part one done."));
        messages.push(user("Second"));
    }
    compact(&mut agent);
    let first = agent.transport_messages();
    let first_record = first
        .iter()
        .find(|message| message.get(ANCHOR).is_some())
        .unwrap()
        .get("content")
        .cloned();
    {
        let mut messages = agent.messages.lock().unwrap();
        messages.extend(read_exchange("b", "b.js", &"y".repeat(200_000)));
        messages.push(answer("Part two done."));
        messages.push(user("Third"));
    }
    compact(&mut agent);
    assert_eq!(agent.context.digests.len(), 2);
    let second = agent.transport_messages();
    let anchors = second
        .iter()
        .filter(|message| message.get(ANCHOR).is_some())
        .count();
    assert_eq!(anchors, 1, "only the end of the records is marked");
    let position = second
        .iter()
        .position(|message| message.get("content").cloned() == first_record)
        .expect("the first record stays byte-identical");
    assert_eq!(position, 1, "right after the system message");
    assert_eq!(second[0], first[0]);
    assert!(
        second[position + 1].get(ANCHOR).is_some(),
        "new records follow the old ones"
    );
}

#[test]
fn records_survive_resume_without_being_stored_in_the_context() {
    let (mut agent, _fixture, _directory) = setup(vec![]);
    let home = Directory::new();
    agent.enable_sessions(home.path()).unwrap();
    agent.prepare_turn("First").unwrap();
    {
        let mut messages = agent.messages.lock().unwrap();
        messages.extend(read_exchange("a", "a.js", &"x".repeat(200_000)));
        messages.push(answer("Root cause recorded once."));
        messages.push(user("Second"));
    }
    compact(&mut agent);
    let handle = agent.sessions().unwrap();
    let saved = handle.store().fixture_load(&handle.id()).unwrap();
    assert_eq!(saved.context.digests.len(), 1);
    assert!(
        !saved
            .context
            .value()
            .encode()
            .contains("Root cause recorded")
    );
    let messages = agent.messages.lock().unwrap().clone();
    assert_eq!(saved.messages, messages);
    assert_eq!(
        saved.context.project(&saved.messages, 0)[1],
        agent.context.project(&messages, 0)[1],
        "the same bytes reach the provider after a resume"
    );
}

#[test]
fn a_model_summary_folds_the_records_when_they_cannot_make_room() {
    let (mut agent, fixture, _directory) = setup(vec![
        (
            200,
            completion("Folded summary of the earlier work.", vec![])
        );
        6
    ]);
    agent.prepare_turn("First").unwrap();
    {
        let mut messages = agent.messages.lock().unwrap();
        messages.extend(read_exchange("a", "a.js", &"x".repeat(200_000)));
        messages.push(answer("Part one done."));
        messages.push(user("Second"));
    }
    compact(&mut agent);
    assert_eq!(agent.context.digests.len(), 1);
    {
        let mut messages = agent.messages.lock().unwrap();
        // User requests stay in the records, so this much cannot fit a small window.
        for index in 0..240 {
            messages.push(user(&format!("Request {index}: {}", "r".repeat(400))));
            messages.push(answer(&format!("Answer {index}")));
        }
        messages.push(user("Latest"));
    }
    let small = Limits {
        context: 30_000,
        output: None,
    };
    agent
        .compact_if_needed(Some(small), true, &mut |_| Ok(()))
        .unwrap();
    assert!(agent.context.digests.is_empty());
    assert_eq!(agent.context.summary, "Folded summary of the earlier work.");
    let requests = fixture.finish_received();
    assert!(!requests.is_empty());
    assert!(
        requests[0].body.encode().contains("read a.js (10 lines)"),
        "the summary request reads the records"
    );
}
