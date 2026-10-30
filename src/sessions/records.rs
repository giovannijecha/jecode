use super::{Document, document::string};
use crate::{
    conversation::ToolCall,
    events::tool_summary,
    json::{self, Value},
};
use std::collections::{BTreeSet, HashMap};

pub enum Record {
    Question {
        request: crate::ask::Request,
    },
    Collaboration {
        event: Value,
    },
    TurnStart,
    Text {
        role: String,
        text: String,
    },
    Tool {
        id: String,
        name: String,
        arguments: Value,
        summary: String,
        result: Value,
        change: Option<crate::file_change::Change>,
    },
    Local {
        command: String,
        result: String,
        kind: String,
        details: Vec<(String, String)>,
    },
}

impl Document {
    pub fn records(&self) -> Vec<Record> {
        let mut records = Vec::new();
        let mut events = vec![Vec::new(); self.messages.len() + 1];
        for event in &self.events {
            if let Some(bucket) = event
                .get("after_message")
                .and_then(Value::as_usize)
                .and_then(|index| events.get_mut(index))
            {
                bucket.push(event);
            }
        }
        let mut calls = HashMap::new();
        let mut display_numbers = HashMap::new();
        let mut latest_tool = None;
        for (index, events) in events.into_iter().enumerate() {
            for event in events {
                if hidden(event, index, &self.context.returned) {
                    continue;
                }
                if event.get("type").and_then(Value::as_str) == Some("collaboration")
                    && let Some(records) = event.get("records").and_then(Value::as_array)
                {
                    for record in records {
                        if let (Some(id), Some(number)) = (
                            record.get("id").and_then(Value::as_str),
                            record.get("number").and_then(Value::as_usize),
                        ) && number > 0
                        {
                            display_numbers.insert(id.to_owned(), number);
                        }
                    }
                }
                append_event(&mut records, event, latest_tool);
            }
            let Some(message) = self.messages.get(index) else {
                break;
            };
            if self.context.returned.contains(&index) {
                continue;
            }
            let role = string(message, "role").unwrap();
            if matches!(role, "user" | "assistant") {
                let text = message.get("content").and_then(Value::as_str).unwrap_or("");
                if !text.trim().is_empty() {
                    records.push(Record::Text {
                        role: role.into(),
                        text: text.into(),
                    });
                }
            }
            if let Some(batch) = message.get("tool_calls").and_then(Value::as_array) {
                for call in batch {
                    let call = ToolCall::parse(call).unwrap();
                    calls.insert(call.id.clone(), call);
                }
            }
            if role == "tool" {
                let id = string(message, "tool_call_id").unwrap();
                let call = calls.remove(id).unwrap();
                let name = call.name.as_str();
                let arguments = call.arguments.as_str();
                let mut arguments =
                    json::parse(arguments).unwrap_or_else(|_| Value::string(arguments));
                if name == "agent"
                    && let Value::Object(fields) = &mut arguments
                {
                    fields.remove("display_number");
                }
                if name == "agent"
                    && let Some(id) = arguments.get("id").and_then(Value::as_str)
                    && let Some(number) = display_numbers.get(id)
                    && let Value::Object(fields) = &mut arguments
                {
                    fields.insert("display_number".into(), Value::number(*number));
                }
                let result = json::parse(string(message, "content").unwrap()).unwrap();
                latest_tool = Some(records.len());
                records.push(Record::Tool {
                    id: id.into(),
                    name: name.into(),
                    arguments,
                    summary: tool_summary(name, &result),
                    result,
                    change: None,
                });
            }
        }
        records
    }
}

fn hidden(event: &Value, index: usize, returned: &BTreeSet<usize>) -> bool {
    let returned_turn = || {
        index
            .checked_sub(1)
            .is_some_and(|last| returned.contains(&last))
    };
    match event.get("type").and_then(Value::as_str) {
        Some("turn_start") => returned.contains(&index),
        Some("turn_error") => returned_turn(),
        Some("local_command")
            if matches!(
                event.get("command").and_then(Value::as_str),
                Some("Context compaction" | "Context previews")
            ) =>
        {
            returned_turn()
        }
        _ => false,
    }
}

fn append_event(records: &mut Vec<Record>, event: &Value, latest_tool: Option<usize>) {
    match event.get("type").and_then(Value::as_str) {
        Some("user_question") => {
            if let Ok(request) = crate::ask::Request::from_event(event) {
                records.push(Record::Question { request });
            }
        }
        Some("collaboration") => records.push(Record::Collaboration {
            event: event.clone(),
        }),
        Some("turn_start") => records.push(Record::TurnStart),
        Some("local_command") => records.push(Record::Local {
            command: string(event, "command").unwrap().into(),
            result: string(event, "result").unwrap().into(),
            kind: event
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("notice")
                .into(),
            details: event
                .get("details")
                .and_then(Value::as_array)
                .unwrap_or(&[])
                .iter()
                .map(|row| {
                    let row = row.as_array().unwrap();
                    (
                        row[0].as_str().unwrap().into(),
                        row[1].as_str().unwrap().into(),
                    )
                })
                .collect(),
        }),
        Some("turn_error") => {
            let partial = string(event, "partial_text").unwrap();
            if !partial.trim().is_empty() {
                records.push(Record::Text {
                    role: "assistant".into(),
                    text: partial.into(),
                });
                records.push(Record::Text {
                    role: "warning".into(),
                    text: "Partial response · incomplete".into(),
                });
            }
            records.push(Record::Text {
                role: event
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .into(),
                text: string(event, "error").unwrap().into(),
            });
        }
        Some("tool_change") => {
            let Some(id) = event.get("tool_call_id").and_then(Value::as_str) else {
                return;
            };
            // Validation ties the event to the immediately preceding tool message.
            // Local events may have appended other records at the same boundary.
            if let Some(Record::Tool {
                id: candidate,
                change,
                ..
            }) = latest_tool.and_then(|index| records.get_mut(index))
                && candidate == id
            {
                *change = event
                    .get("change")
                    .and_then(|value| crate::file_change::Change::from_value(value).ok());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
