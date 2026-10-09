use super::{Document, document::string};
use crate::{
    events::tool_summary,
    json::{self, Value},
};
use std::collections::HashMap;

pub enum Record {
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
        for (index, events) in events.into_iter().enumerate() {
            for event in events {
                append_event(&mut records, event);
            }
            let Some(message) = self.messages.get(index) else {
                break;
            };
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
                    calls.insert(string(call, "id").unwrap(), call);
                }
            }
            if role == "tool" {
                let id = string(message, "tool_call_id").unwrap();
                let call = calls.remove(id).unwrap();
                let function = call.get("function").unwrap();
                let name = string(function, "name").unwrap();
                let arguments = string(function, "arguments").unwrap();
                let result = json::parse(string(message, "content").unwrap()).unwrap();
                records.push(Record::Tool {
                    id: id.into(),
                    name: name.into(),
                    arguments: json::parse(arguments).unwrap_or_else(|_| Value::string(arguments)),
                    summary: tool_summary(name, &result),
                    result,
                });
            }
        }
        records
    }
}

fn append_event(records: &mut Vec<Record>, event: &Value) {
    match event.get("type").and_then(Value::as_str) {
        Some("local_command")
            if event.get("receipt").and_then(Value::as_str) != Some("report_delivery") =>
        {
            records.push(Record::Local {
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
            })
        }
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
        _ => {}
    }
}

#[cfg(test)]
mod tests;
