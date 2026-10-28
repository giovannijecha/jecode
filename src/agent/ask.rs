use super::Agent;
use crate::{
    ask::{Question, Request},
    backend::ToolCall,
    events::{Event, EventSink},
    json::{self, Value},
    sessions::Stage,
};

impl Agent {
    pub fn has_pending_question(&self) -> bool {
        self.questions.current().is_some()
    }

    pub(super) fn ask_tool(&self, call: &ToolCall, events: &mut impl EventSink) -> Value {
        if self.child {
            return unavailable("Only the main agent can ask the user");
        }
        let question = match json::parse(&call.arguments).and_then(|value| {
            crate::tools::parallel(&value)?;
            Question::parse(&self.redactor.value(&value))
        }) {
            Ok(question) => question,
            Err(error) => return Value::object([("error", Value::string(error))]),
        };
        let request = Request {
            id: call.id.clone(),
            question,
        };
        match self.questions.begin(request.clone()) {
            Ok(true) => {}
            Ok(false) => return unavailable("Interactive input is unavailable"),
            Err(error) => return Value::object([("error", Value::string(error))]),
        }
        if let Err(error) = events.emit(Event::UserQuestion { request }) {
            return Value::object([("error", Value::string(error))]);
        }
        self.questions.wait(&self.cancellation).map_or_else(
            || Value::object([("outcome", Value::string("cancelled"))]),
            |reply| reply.result(),
        )
    }

    /// Finish a saved, unresolved ask_user call before making the next model
    /// request. The original assistant call stays in the transcript.
    pub fn continue_question(&mut self, events: &mut impl EventSink) -> Result<bool, String> {
        let Some(request) = self.questions.current() else {
            return Ok(false);
        };
        let call = self.pending_question_call(&request.id)?;
        let result = match self.questions.saved_reply() {
            Some(reply) => reply.result(),
            None => {
                events.emit(Event::UserQuestion {
                    request: request.clone(),
                })?;
                self.questions.wait(&self.cancellation).map_or_else(
                    || Value::object([("outcome", Value::string("cancelled"))]),
                    |reply| reply.result(),
                )
            }
        };
        if self.questions.is_suspended() {
            return self.checkpoint(Stage::Preserve).map(|()| true);
        }
        self.tool_result(&call, &result);
        self.questions.complete(&call.id);
        self.checkpoint(Stage::ToolFinished)?;
        events.emit(self.finished_event(&call, &result))?;
        if self.cancellation.requested() {
            self.checkpoint(Stage::Ready)?;
            return Err("Operation cancelled".into());
        }
        self.refresh_project_instructions()?;
        self.refresh_skills();
        let result = self.drive_turn(events);
        if self.questions.is_suspended() {
            return self.checkpoint(Stage::Preserve).map(|()| true);
        }
        if let Err(error) = &result {
            self.record_turn_error(error, "");
        }
        let saved = self.checkpoint(Stage::Ready);
        result.and(saved).map(|()| true)
    }

    fn pending_question_call(&self, id: &str) -> Result<ToolCall, String> {
        let messages = self.messages.lock().unwrap();
        for message in messages.iter().rev() {
            if message.get("role").and_then(Value::as_str) == Some("assistant") {
                let calls = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .ok_or("Saved question has no tool calls")?;
                return calls
                    .iter()
                    .map(ToolCall::parse)
                    .find_map(|call| match call {
                        Ok(call) if call.id == id && call.name == "ask_user" => Some(Ok(call)),
                        Err(error) => Some(Err(error)),
                        _ => None,
                    })
                    .unwrap_or_else(|| Err("Saved question call is missing".into()));
            }
        }
        Err("Saved question has no assistant call".into())
    }
}

fn unavailable(reason: &str) -> Value {
    Value::object([
        ("outcome", Value::string("unavailable")),
        ("reason", Value::string(reason)),
    ])
}

#[cfg(test)]
mod tests;
