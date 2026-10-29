use super::Agent;
use crate::{json::Value, todo::Plan};

impl Agent {
    /// The current plan from successful, durable tool results in this conversation.
    pub fn todo_plan(&self) -> Option<Plan> {
        crate::todo::latest(&self.messages.lock().unwrap())
    }

    pub(super) fn todo_tool(&self, arguments: &Value) -> Result<Value, String> {
        match arguments.get("action").and_then(Value::as_str) {
            Some("get") => Ok(self.todo_plan().unwrap_or_else(Plan::empty).value()),
            Some("set") => {
                let previous = self.todo_plan();
                Ok(Plan::set(arguments, previous.as_ref())?.value())
            }
            _ => Err("action must be get or set".into()),
        }
    }
}

#[cfg(test)]
mod tests;
