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
        // Root exit and final completion are separate rows; each is saved
        // once when both the UI and the worker observe it.
        let process_kind = event
            .get("type")
            .and_then(crate::json::Value::as_str)
            .filter(|kind| matches!(*kind, "process_root_exited" | "process_finished"));
        let process_id =
            process_kind.and_then(|_| event.get("process_id").and_then(crate::json::Value::as_str));
        let existing = process_id.and_then(|id| {
            events.iter().position(|saved| {
                saved.get("type").and_then(crate::json::Value::as_str) == process_kind
                    && saved.get("process_id").and_then(crate::json::Value::as_str) == Some(id)
            })
        });
        let mut displaced = None;
        if let Some(index) = existing {
            if index < insertion {
                return Ok(events[index].clone());
            }
            // It is in the unsaved worker tail. Move it into the validated
            // prefix so a crash before the next checkpoint cannot lose it.
            displaced = Some(events.remove(index));
        }
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
            if let (Some(index), Some(original)) = (existing, displaced) {
                events.insert(index, original);
            }
            let mut live = self.0.live.lock().unwrap();
            live.document.events.pop();
            live.revision = live.revision.wrapping_add(1);
        }
        result.map(|()| event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::Routing,
        effort::Effort,
        json::Value,
        redact::Redactor,
        sessions::{Document, Stage, Store},
        test_support::Directory,
    };
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_worker_tail_and_a_ui_receipt_share_one_durable_process_event() {
        let home = Directory::new();
        let project = Directory::new();
        let store = Store::new(home.path().to_path_buf(), project.path()).unwrap();
        let mut document = Document::new(
            project.path().to_path_buf(),
            "fixture/model".into(),
            Effort::High,
        )
        .unwrap();
        document.messages.push(Value::object([
            ("role", Value::string("system")),
            ("content", Value::string("fixture")),
        ]));
        let redactor = Redactor::empty();
        let archive = Archive {
            backend: "openrouter".into(),
            model: "fixture/model".into(),
            directory: project.path().to_path_buf(),
            messages: Arc::new(Mutex::new(document.messages.clone())),
            redactor: redactor.clone(),
            effort: "high".into(),
            events: Arc::new(Mutex::new(vec![])),
        };
        let handle = Handle::new(store, document, redactor);
        handle
            .checkpoint(
                &archive,
                0,
                &crate::context::Context::default(),
                &Routing::Default,
                false,
                Stage::Ready,
            )
            .unwrap();
        let event = Value::object([
            ("type", Value::string("process_finished")),
            ("process_id", Value::string("process-stable")),
            ("status", Value::string("exited")),
            ("after_message", Value::number(1)),
        ]);
        archive.events.lock().unwrap().push(event.clone());
        let mut visible = event;
        if let Value::Object(fields) = &mut visible {
            fields.insert("after_tool".into(), Value::string("hold"));
        }
        handle.append_event(&archive, visible.clone()).unwrap();
        handle.append_event(&archive, visible).unwrap();
        let root = Value::object([
            ("type", Value::string("process_root_exited")),
            ("process_id", Value::string("process-stable")),
            ("command", Value::string("launcher")),
            ("status", Value::string("running")),
            ("after_message", Value::number(1)),
            (
                "root_process",
                Value::object([
                    ("pid", Value::number(9)),
                    ("status", Value::string("exited")),
                    ("exit_code", Value::number(7)),
                    ("exited_after_ms", Value::number(100)),
                ]),
            ),
        ]);
        handle.append_event(&archive, root.clone()).unwrap();
        handle.append_event(&archive, root).unwrap();
        let saved = handle.snapshot();
        assert_eq!(saved.events.len(), 2);
        assert_eq!(archive.events.lock().unwrap().len(), 2);
        assert_eq!(
            saved.events[0].get("after_tool").and_then(Value::as_str),
            Some("hold")
        );
        assert_eq!(
            saved.events[1].get("type").and_then(Value::as_str),
            Some("process_root_exited")
        );
    }
}
