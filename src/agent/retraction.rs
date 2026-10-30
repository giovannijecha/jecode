use super::Agent;
use crate::{context::Context, json::Value};

/// Context before accepting a turn's first request. Original history stays in
/// the journal when input returns to the composer.
pub struct TurnInput {
    from: usize,
    context: Context,
}

impl Agent {
    pub fn input_checkpoint(&self) -> TurnInput {
        TurnInput {
            from: self.messages.lock().unwrap().len(),
            context: self.context.clone(),
        }
    }

    /// Withdraws only unanswered input. The interface must capture the restored
    /// drafts before saving, so their ownership changes in one checkpoint.
    pub fn return_input(&mut self, input: TurnInput) -> bool {
        let messages = self.messages.lock().unwrap();
        let end = messages.len();
        if input.from >= end
            || messages[input.from..]
                .iter()
                .any(|message| message.get("role").and_then(Value::as_str) != Some("user"))
        {
            return false;
        }
        drop(messages);
        let reported = std::mem::take(&mut self.context.reported);
        self.context = input.context;
        self.context.reported = reported;
        self.context.returned.extend(input.from..end);
        self.context.reset_projection();
        self.events.lock().unwrap().push(Value::object([
            ("type", Value::string("turn_returned")),
            ("from_message", Value::number(input.from)),
            ("after_message", Value::number(end)),
        ]));
        true
    }
}
