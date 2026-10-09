use super::Agent;
use crate::{
    effort::Effort,
    json::Value,
    sessions::{Document, Handle, Input, Stage, Store},
};
use std::path::Path;

impl Agent {
    pub fn enable_sessions(&mut self, home: &Path) -> Result<(), String> {
        if self.persistence.is_none() {
            let store = Store::new(home.to_path_buf(), self.tools.root())?;
            let document = Document::new(
                self.tools.root().to_path_buf(),
                self.model().into(),
                self.effort(),
            )?;
            self.tools.configure_session_output(
                store.output_directory(),
                &document.id,
                self.redactor.clone(),
            )?;
            self.tools
                .configure_temporary(store.temporary_area(&document.id)?);
            self.tools.configure_attachments(store.attachments());
            self.context.reset_usage();
            self.persistence = Some(Handle::new(store, document, self.redactor.clone()));
            self.checkpoint(Stage::Preserve)?;
        }
        Ok(())
    }

    /// Copies files into the session attachment pool, all or none.
    pub fn import_attachments(
        &self,
        paths: &[std::path::PathBuf],
    ) -> Result<Vec<crate::attachments::Attachment>, String> {
        let pool = self
            .tools
            .attachments()
            .ok_or("Attachments need session storage, which is unavailable")?;
        paths
            .iter()
            .map(|path| pool.import_file(path, &self.cancellation))
            .collect()
    }

    pub fn sessions(&self) -> Option<Handle> {
        self.persistence.clone()
    }

    pub(super) fn checkpoint(&self, stage: Stage) -> Result<(), String> {
        match &self.persistence {
            Some(handle) => {
                handle.checkpoint(&self.archive(), self.compatible_from, &self.context, stage)
            }
            None => Ok(()),
        }
    }

    pub fn save_session(&self) -> Result<(), String> {
        self.checkpoint(Stage::Preserve)
    }

    pub fn prepare_turn(
        &mut self,
        prompt: impl Into<crate::attachments::Prompt>,
    ) -> Result<(), String> {
        let prompt = prompt.into();
        if prompt.is_empty() {
            return Err("The prompt must not be empty".into());
        }
        if self.prepared.is_some() {
            return Err("A prompt is already prepared".into());
        }
        self.refresh_project_instructions()?;
        let before = self.messages.lock().unwrap().len();
        self.messages.lock().unwrap().push(prompt.message());
        if let Err(error) = self.checkpoint(Stage::Begin) {
            self.messages.lock().unwrap().truncate(before);
            return Err(error);
        }
        self.prepared = Some(prompt);
        Ok(())
    }

    pub fn start_new(&mut self, model: String, effort: Effort) -> Result<(), String> {
        crate::openrouter::validate_model(&model)?;
        self.save_session()?;
        let next = if let Some(old) = &self.persistence {
            let input = old.snapshot().input;
            let mut document =
                Document::new(self.tools.root().to_path_buf(), model.clone(), effort)?;
            document.input = input.clone();
            document.messages = vec![self.system_message()];
            let next = Handle::new(old.store(), document, self.redactor.clone());
            next.flush()?;
            old.input(Input {
                history: input.history.clone(),
                ..Input::default()
            });
            if let Err(error) = old.flush() {
                old.input(input);
                return Err(error);
            }
            Some(next)
        } else {
            None
        };
        let temporary = next
            .as_ref()
            .map(|handle| handle.store().temporary_area(&handle.id()))
            .transpose()?;
        self.client.set_model(model)?;
        self.client.set_effort(effort);
        self.clear();
        self.persistence = next;
        if let Some(handle) = &self.persistence {
            self.tools.configure_session_output(
                handle.store().output_directory(),
                &handle.id(),
                self.redactor.clone(),
            )?;
        }
        if let Some(temporary) = temporary {
            self.tools.configure_temporary(temporary);
        }
        Ok(())
    }

    pub(super) fn record_turn_error(&self, error: &str, partial: &str) {
        self.events.lock().unwrap().push(Value::object([
            ("type", Value::string("turn_error")),
            ("error", Value::string(self.redact(error))),
            (
                "kind",
                Value::string(if self.cancellation.requested() {
                    "warning"
                } else {
                    "error"
                }),
            ),
            ("partial_text", Value::string(partial)),
            (
                "after_message",
                Value::number(self.messages.lock().unwrap().len()),
            ),
        ]));
    }

    pub fn resume(&mut self, id: &str) -> Result<String, String> {
        let current = self
            .persistence
            .as_ref()
            .ok_or("Session storage is unavailable")?;
        if current.id() == id {
            return Err("This session is already open".into());
        }
        self.save_session()?;
        let (handle, status) = Handle::open(current.store(), id, self.redactor.clone())?;
        let document = handle.snapshot();
        let temporary = handle.store().temporary_area(&document.id)?;
        self.client.set_model(document.model)?;
        self.client.set_effort(document.effort);
        *self.messages.lock().unwrap() = document.messages;
        *self.events.lock().unwrap() = document.events;
        self.compatible_from = document.compatible_from;
        self.context = document.context;
        let calibration = self.context.calibration;
        self.context.reset_usage();
        // A fresh request path invalidates the direct prompt measurement,
        // while the saved model's conservative token density remains useful.
        self.context.calibration = calibration;
        self.prepared = None;
        self.project_instructions.clear();
        self.cancellation.reset();
        self.persistence = Some(handle);
        let handle = self.persistence.as_ref().unwrap();
        self.tools.configure_session_output(
            handle.store().output_directory(),
            &handle.id(),
            self.redactor.clone(),
        )?;
        self.tools.configure_temporary(temporary);
        Ok(status)
    }

    pub(crate) fn delete_session(
        &mut self,
        id: &str,
        model: String,
        effort: Effort,
    ) -> Result<(crate::sessions::DeleteReport, bool), String> {
        let current = self
            .persistence
            .as_ref()
            .ok_or("Session storage is unavailable")?;
        if current.id() != id {
            return current.store().delete(id).map(|report| (report, false));
        }
        if self.prepared.is_some() || current.active() {
            return Err("Session deletion is available only when the turn is ready".into());
        }
        crate::openrouter::validate_model(&model)?;
        let store = current.store();
        let mut document = Document::new(self.tools.root().to_path_buf(), model.clone(), effort)?;
        document.input = current.snapshot().input;
        document.input.history.clear();
        let temporary = store.temporary_area(&document.id)?;
        let outputs = crate::output::Store::for_session(
            store.output_directory(),
            self.tools.root().to_path_buf(),
            &document.id,
            self.redactor.clone(),
        )?;
        let next = Handle::new(store, document, self.redactor.clone());
        let live_attachments = next.snapshot().input.attachment_ids();
        let report = current.delete(&live_attachments)?;
        self.client.set_model(model)?;
        self.client.set_effort(effort);
        self.clear();
        self.cancellation.reset();
        self.persistence = Some(next);
        self.tools.configure_output_store(outputs);
        self.tools.configure_temporary(temporary);
        Ok((report, true))
    }
}
