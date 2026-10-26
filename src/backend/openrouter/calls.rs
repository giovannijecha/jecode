//! OpenRouter's function-call envelope around canonical tool calls.

use crate::conversation::ToolCall;
use crate::json::Value;

const NATIVE: &str = "jecode_native_tool_calls";

pub(super) fn request(mut messages: Vec<Value>) -> Result<Vec<Value>, String> {
    for message in &mut messages {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Value::Object(fields) = message else {
            continue;
        };
        let native = fields.remove(NATIVE);
        if let (Some(Value::Array(native)), Some(Value::Array(calls))) =
            (&native, fields.get("tool_calls"))
            && native.len() == calls.len()
            && native.iter().zip(calls).all(|(native, call)| {
                ToolCall::parse(native)
                    .ok()
                    .zip(ToolCall::parse(call).ok())
                    .is_some_and(|(native, call)| native == call)
            })
        {
            fields.insert("tool_calls".into(), Value::Array(native.clone()));
            continue;
        }
        let Some(Value::Array(calls)) = fields.get_mut("tool_calls") else {
            continue;
        };
        for call in calls {
            let parsed = ToolCall::parse(call)?;
            if call.get("function").is_none() {
                *call = wire(&parsed);
            }
        }
    }
    Ok(messages)
}

pub(super) fn response(message: &mut Value, calls: &[ToolCall]) {
    let Value::Object(fields) = message else {
        return;
    };
    if calls.is_empty() {
        fields.remove("tool_calls");
        fields.remove(NATIVE);
    } else {
        if let Some(Value::Array(entries)) = fields.get("tool_calls").cloned()
            && entries.len() == calls.len()
            && entries.iter().all(|entry| entry.get("function").is_some())
            && entries
                .iter()
                .zip(calls)
                .any(|(entry, call)| *entry != wire(call))
        {
            fields.insert(NATIVE.into(), Value::Array(entries));
        }
        fields.insert(
            "tool_calls".into(),
            Value::Array(calls.iter().map(ToolCall::value).collect()),
        );
    }
}

fn wire(call: &ToolCall) -> Value {
    Value::object([
        ("id", Value::string(&call.id)),
        ("type", Value::string("function")),
        (
            "function",
            Value::object([
                ("name", Value::string(&call.name)),
                ("arguments", Value::string(&call.arguments)),
            ]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_calls_use_function_wire_shape_and_stay_canonical_after_response() {
        let call = ToolCall {
            id: "call-1".into(),
            name: "read".into(),
            arguments: r#"{"path":"src/main.rs","work":"Inspect source"}"#.into(),
        };
        let mut message = Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::Null),
            ("tool_calls", Value::Array(vec![call.value()])),
        ]);
        let wire = request(vec![message.clone()]).unwrap();
        assert_eq!(
            wire[0].get("tool_calls").and_then(Value::as_array).unwrap()[0]
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str),
            Some("read")
        );
        assert_eq!(
            wire[0].get("tool_calls").and_then(Value::as_array).unwrap()[0]
                .get("function")
                .and_then(|function| function.get("arguments"))
                .and_then(Value::as_str),
            Some(call.arguments.as_str())
        );
        response(&mut message, std::slice::from_ref(&call));
        assert_eq!(
            message.get("tool_calls").and_then(Value::as_array).unwrap()[0],
            call.value()
        );
    }

    #[test]
    fn legacy_function_calls_remain_replayable() {
        let legacy = Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![wire(&ToolCall {
                    id: "old".into(),
                    name: "shell".into(),
                    arguments: "{}".into(),
                })]),
            ),
        ]);
        let projected = request(vec![legacy]).unwrap();
        assert_eq!(
            projected[0]
                .get("tool_calls")
                .and_then(Value::as_array)
                .unwrap()[0]
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str),
            Some("shell")
        );
    }

    #[test]
    fn native_call_metadata_survives_canonical_storage_and_same_backend_replay() {
        let call = ToolCall {
            id: "one".into(),
            name: "read".into(),
            arguments: "{}".into(),
        };
        let mut native = wire(&call);
        if let Value::Object(fields) = &mut native {
            fields.insert("provider_extension".into(), Value::string("keep"));
        }
        let mut message = Value::object([
            ("role", Value::string("assistant")),
            ("tool_calls", Value::Array(vec![native.clone()])),
        ]);
        response(&mut message, std::slice::from_ref(&call));
        assert_eq!(
            message.get("tool_calls").and_then(Value::as_array).unwrap()[0],
            call.value()
        );
        let replayed = request(vec![message]).unwrap();
        assert_eq!(
            replayed[0]
                .get("tool_calls")
                .and_then(Value::as_array)
                .unwrap()[0],
            native
        );
        assert!(replayed[0].get(NATIVE).is_none());
    }
}
