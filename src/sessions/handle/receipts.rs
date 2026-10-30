//! External receipts appended while the agent worker may own the turn.

use super::Handle;
use crate::export::Archive;
use std::time::Instant;

impl Handle {
    /// Append an external receipt while the agent worker may own the turn.
    /// Keep the archive mirror and journal in the same order as checkpoints.
    pub fn append_event(
        &self,
        archive: &Archive,
        event: crate::json::Value,
    ) -> Result<crate::json::Value, String> {
        // The same lock order as checkpoints: disk access, then history.
        let _io = self.0.io.lock().unwrap();
        let messages = archive.messages.lock().unwrap();
        let mut events = archive.events.lock().unwrap();
        let mut live = self.0.live.lock().unwrap();
        if live.deleted {
            return Err("This session was deleted; start a new conversation".into());
        }
        live.redactor.include(archive.redactor.clone());
        if messages.len() < live.document.messages.len()
            || events.len() < live.document.events.len()
        {
            return Err("Session history cannot shrink; start a new conversation instead".into());
        }
        // A tool result can be in the archive before ToolFinished updates the
        // pending context. Persist this receipt at the last validated prefix;
        // the next main checkpoint will append the in-flight messages/events.
        let insertion = live.document.events.len();
        let mut event = live.redactor.value(&event);
        let crate::json::Value::Object(fields) = &mut event else {
            return Err("External session event must be an object".into());
        };
        fields.insert(
            "after_message".into(),
            crate::json::Value::number(live.document.messages.len()),
        );
        events.insert(insertion, event.clone());
        live.document.events.push(event.clone());
        live.dirty.get_or_insert_with(Instant::now);
        live.revision = live.revision.wrapping_add(1);
        drop(live);
        let result = self.save_locked_io();
        if result.is_err() {
            events.remove(insertion);
            let mut live = self.0.live.lock().unwrap();
            live.document.events.pop();
            live.revision = live.revision.wrapping_add(1);
        }
        result.map(|()| event)
    }
}
