use super::{Reply, Request};
use crate::{conversation::ToolCall, json::Value};
use std::collections::BTreeSet;

/// Only the unresolved tail can resume. Provider call IDs may be reused in
/// later turns, so an old receipt with the same ID is insufficient evidence.
pub(crate) fn recovery(messages: &[Value], events: &[Value]) -> Option<(Request, Option<Reply>)> {
    let mut answered = BTreeSet::new();
    let (boundary, calls) = messages
        .iter()
        .enumerate()
        .rev()
        .find_map(
            |(index, message)| match message.get("role").and_then(Value::as_str) {
                Some("tool") => {
                    if let Some(id) = message.get("tool_call_id").and_then(Value::as_str) {
                        answered.insert(id);
                    }
                    None
                }
                _ => Some((
                    index + 1,
                    message.get("tool_calls").and_then(Value::as_array),
                )),
            },
        )?;
    let calls = calls?;
    let unresolved: BTreeSet<_> = calls
        .iter()
        .filter_map(|call| ToolCall::parse(call).ok())
        .filter(|call| call.name == "ask_user" && !answered.contains(call.id.as_str()))
        .map(|call| call.id)
        .collect();
    let mut restored: Option<(Request, Option<Reply>)> = None;
    for event in events {
        if event
            .get("after_message")
            .and_then(Value::as_usize)
            .unwrap_or(0)
            < boundary
        {
            continue;
        }
        let Some(id) = event.get("tool_call_id").and_then(Value::as_str) else {
            continue;
        };
        if !unresolved.contains(id) {
            continue;
        }
        match event.get("type").and_then(Value::as_str) {
            Some("user_question") => {
                restored = Some((Request::from_event(event).ok()?, None));
            }
            Some("user_answer") => {
                if let Some((request, reply)) = &mut restored
                    && request.id == id
                {
                    *reply = Reply::parse(event.get("result")?)
                        .filter(|reply| matches(request, reply).is_ok());
                }
            }
            _ => {}
        }
    }
    restored
}

pub(crate) fn validate_answer(event: &Value, events: &[Value]) -> Result<(), String> {
    let id = event
        .get("tool_call_id")
        .and_then(Value::as_str)
        .ok_or("Missing answer call ID")?;
    let result = event.get("result").ok_or("Missing saved answer")?;
    let reply = Reply::parse(result).ok_or("Invalid saved answer")?;
    let question = events
        .iter()
        .rev()
        .find(|event| {
            event.get("type").and_then(Value::as_str) == Some("user_question")
                && event.get("tool_call_id").and_then(Value::as_str) == Some(id)
        })
        .ok_or("Saved answer has no question")?;
    matches(&Request::from_event(question)?, &reply)
}

fn matches(request: &Request, reply: &Reply) -> Result<(), String> {
    match reply {
        Reply::Answered(answers) => request
            .check(answers)
            .map_err(|error| format!("Invalid saved answer: {error}")),
        Reply::Dismissed => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call() -> Value {
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![
                    ToolCall {
                        id: "reused".into(),
                        name: "ask_user".into(),
                        arguments: "{}".into(),
                    }
                    .value(),
                ]),
            ),
        ])
    }

    fn question() -> Value {
        Value::object([
            ("type", Value::string("user_question")),
            ("tool_call_id", Value::string("reused")),
            ("after_message", Value::number(1)),
            (
                "question",
                Value::object([
                    ("title", Value::string("Scope")),
                    ("question", Value::string("Which records?")),
                    (
                        "options",
                        Value::Array(vec![Value::object([
                            ("id", Value::string("all")),
                            ("label", Value::string("All")),
                        ])]),
                    ),
                ]),
            ),
        ])
    }

    fn answer(result: Value) -> Value {
        Value::object([
            ("type", Value::string("user_answer")),
            ("tool_call_id", Value::string("reused")),
            ("after_message", Value::number(1)),
            ("result", result),
        ])
    }

    #[test]
    fn resolved_or_reused_call_id_does_not_revive_an_old_question() {
        let events = vec![question()];
        let mut messages = vec![call()];
        assert!(recovery(&messages, &events).is_some());
        messages.push(Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string("reused")),
            ("content", Value::string("{\"outcome\":\"dismissed\"}")),
        ]));
        assert!(recovery(&messages, &events).is_none());
        messages.push(call());
        assert!(
            recovery(&messages, &events).is_none(),
            "old question cannot represent a new unstarted call"
        );
    }

    #[test]
    fn an_earlier_single_question_and_answer_resume_as_one_question() {
        let saved = answer(Value::object([
            ("outcome", Value::string("answered")),
            ("choice", Value::string("all")),
            ("text", Value::string("")),
        ]));
        let events = vec![question(), saved.clone()];
        validate_answer(&saved, &events).unwrap();
        let (request, reply) = recovery(&[call()], &events).unwrap();
        assert_eq!(request.questions.len(), 1);
        assert_eq!(request.questions[0].id, crate::ask::LEGACY_ID);
        assert_eq!(request.subject(), "Scope");
        assert_eq!(
            reply.unwrap(),
            Reply::Answered(vec![crate::ask::Answer {
                id: crate::ask::LEGACY_ID.into(),
                choices: vec!["all".into()],
                text: String::new(),
            }])
        );
    }

    #[test]
    fn malformed_answers_cannot_be_restored_as_human_input() {
        let events = vec![question()];
        for result in [
            Value::object([("outcome", Value::string("answered"))]),
            Value::object([
                ("outcome", Value::string("answered")),
                ("choice", Value::Null),
                ("text", Value::string(" ")),
            ]),
            Value::object([
                ("outcome", Value::string("answered")),
                ("choice", Value::string("missing")),
                ("text", Value::string("")),
            ]),
            Value::object([
                ("outcome", Value::string("answered")),
                ("answers", Value::Array(Vec::new())),
            ]),
        ] {
            assert!(validate_answer(&answer(result), &events).is_err());
        }
        assert!(
            validate_answer(
                &answer(Value::object([("outcome", Value::string("dismissed"))])),
                &events
            )
            .is_ok()
        );
    }
}
