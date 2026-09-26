//! Versioned session storage uses the same output validation as streamed responses.
use super::{Error, Response, Status, response::assemble};
use crate::json::{self, Value};
impl Response {
    pub(crate) fn snapshot(&self) -> Value {
        let mut usage = std::collections::BTreeMap::new();
        for (key, number) in [
            ("input_tokens", self.usage.input),
            ("output_tokens", self.usage.output),
        ] {
            if let Some(number) = number {
                usage.insert(key.into(), Value::Number(number.to_string()));
            }
        }
        if let Some(number) = self.usage.cached {
            usage.insert(
                "input_tokens_details".into(),
                json::object([("cached_tokens", Value::Number(number.to_string()))]),
            );
        }
        if let Some(number) = self.usage.reasoning {
            usage.insert(
                "output_tokens_details".into(),
                json::object([("reasoning_tokens", Value::Number(number.to_string()))]),
            );
        }
        let mut fields = vec![
            ("id", Value::String(self.id.clone())),
            (
                "status",
                Value::String(
                    if self.status == Status::Incomplete {
                        "incomplete"
                    } else {
                        "completed"
                    }
                    .into(),
                ),
            ),
            ("output", Value::Array(self.output.clone())),
            ("usage", Value::Object(usage)),
        ];
        if let Some(end_turn) = self.end_turn {
            fields.push(("end_turn", Value::Bool(end_turn)));
        }
        json::object(fields)
    }
    pub(crate) fn restore(value: &Value) -> Result<Self, Error> {
        let output = value
            .get("output")
            .and_then(Value::array)
            .ok_or(Error::InvalidEvent)?
            .to_vec();
        let complete = match value.get("status").and_then(Value::text) {
            Some("completed") => true,
            Some("incomplete") => false,
            _ => return Err(Error::InvalidEvent),
        };
        assemble(value, output, complete)
    }
}
