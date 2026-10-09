use super::Agent;
use crate::{
    context,
    json::{self, Value},
    openrouter::ToolCall,
};

impl Agent {
    pub(super) fn execute_tool(&self, call: &ToolCall) -> Value {
        if call.name == "read"
            && let Ok(arguments) = json::parse(&call.arguments)
            && arguments
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| path.starts_with("history:"))
        {
            return self
                .read_history(&arguments)
                .unwrap_or_else(|error| Value::object([("error", Value::string(error))]));
        }
        self.tools
            .execute_with_cancel(&call.name, &call.arguments, &self.cancellation)
    }

    fn read_history(&self, arguments: &Value) -> Result<Value, String> {
        let path = arguments
            .get("path")
            .and_then(Value::as_str)
            .ok_or("path must be a string")?;
        let messages = self.messages.lock().unwrap();
        let mut source = None;
        let text = if path == "history:memory" {
            self.context.summary.clone()
        } else if path == "history:requests" {
            messages
                .iter()
                .enumerate()
                .filter(|(_, message)| message.get("role").and_then(Value::as_str) == Some("user"))
                .map(|(index, message)| {
                    format!(
                        "history:{index} — user request:\n{}\n",
                        crate::attachments::provider::user_text(message)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            let id = path.strip_prefix("history:").unwrap();
            if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(
                    "history: requires a zero-based message index, requests or memory".into(),
                );
            }
            let index: usize = id.parse().map_err(|_| "Invalid history message index")?;
            let message = messages
                .get(index)
                .ok_or("History message does not exist in this session")?;
            source = Some(context::source::record(&messages, index));
            if message.get("role").and_then(Value::as_str) == Some("user") {
                crate::attachments::provider::user_text(message)
            } else if message.get("role").and_then(Value::as_str) == Some("tool") {
                let content = message.get("content").and_then(Value::as_str).unwrap_or("");
                json::parse(content).map_or_else(|_| content.to_owned(), |value| value.pretty())
            } else {
                let mut portable = context::portable(message);
                crate::attachments::annotations::history(&mut portable);
                portable.pretty()
            }
        };
        drop(messages);
        let mut page =
            crate::tools::read_text_page(&self.redact(&text), arguments, &self.cancellation)?;
        if let Value::Object(fields) = &mut page {
            fields.insert("history_reference".into(), Value::string(path));
            if let Some(source) = source {
                // The existing alias identifies the tool call, not its user request.
                if let Some(call) = source.get("call_history").and_then(Value::as_str) {
                    fields.insert("request_history_reference".into(), Value::string(call));
                }
                fields.insert("source".into(), source);
            }
        }
        Ok(self.redactor.value(&page))
    }
}
