use super::Agent;
use crate::{json::Value, scratch::Info};

impl Agent {
    pub(super) fn temporary_instructions(&self) -> String {
        let mut instructions = self.tools.temporary_instructions();
        if !instructions.is_empty()
            && let Some(event) = self.events.lock().unwrap().iter().rev().find(|event| {
                event.get("type").and_then(Value::as_str) == Some("temporary_cleanup")
            })
        {
            let boundary = event
                .get("after_message")
                .and_then(Value::as_usize)
                .unwrap_or(0);
            let result = event.get("result").and_then(Value::as_str).unwrap_or("");
            let cleanup = Value::object([
                ("after_message", Value::number(boundary)),
                ("result", Value::string(result)),
                ("current_inventory", Value::string("not_scanned")),
            ]);
            instructions.push_str(&format!(
                "\nNative temporary cleanup:\n{}",
                cleanup.encode()
            ));
        }
        instructions
    }

    pub fn temporary_info(&self) -> Result<Info, String> {
        self.tools.temporary()?.info()
    }

    pub fn clean_temporary(&mut self) -> Result<Info, String> {
        if self.prepared.is_some()
            || self
                .persistence
                .as_ref()
                .is_some_and(|handle| handle.active())
        {
            return Err("Temporary cleanup is available only when the session is ready".into());
        }
        // Retain the conversation before performing the user's explicit deletion.
        self.save_session()?;
        let info = self.temporary_info()?;
        if info.files == 0 && info.directories == 0 {
            return Ok(info);
        }
        self.record_temporary_cleanup(
            "The user requested /tmp clean. Cleanup started and may be incomplete until a completion is recorded; inspect tmp: files before relying on earlier references."
        );
        self.save_session()?;
        let result = self.tools.temporary()?.clean();
        let status = match &result {
            Ok(info) => format!(
                "Cleared {} temporary files and {} directories ({} bytes). Earlier tmp: file references no longer exist.",
                info.files, info.directories, info.bytes
            ),
            Err(error) => format!(
                "Cleanup failed: {error}. Inspect temporary files before relying on earlier tmp: references."
            ),
        };
        self.record_temporary_cleanup(&status);
        let saved = self.save_session();
        match (result, saved) {
            (Ok(info), Ok(())) => Ok(info),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(format!(
                "Temporary files were removed, but autosave failed: {error}. The updated conversation remains in memory."
            )),
        }
    }

    fn record_temporary_cleanup(&mut self, status: &str) {
        let boundary = self.messages.lock().unwrap().len();
        self.events.lock().unwrap().push(Value::object([
            ("type", Value::string("temporary_cleanup")),
            ("result", Value::string(self.redact(status))),
            ("after_message", Value::number(boundary)),
        ]));
        self.context.reset_usage();
    }
}

#[cfg(test)]
mod tests;
