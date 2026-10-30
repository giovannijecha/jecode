use super::{Document, Input, Pending, Store, context, storage::Lease};
use crate::{export::Archive, redact::Redactor};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

mod autosave;
mod receipts;
mod save;

const SAVE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct Handle(Arc<Shared>);

struct Shared {
    live: Mutex<Live>,
    io: Mutex<()>,
    background_save: AtomicBool,
    background_result: Mutex<Option<Result<(), String>>>,
    autosave: Arc<autosave::Autosave>,
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.autosave.stop();
    }
}

struct Live {
    store: Store,
    document: Document,
    lease: Option<Arc<Lease>>,
    redactor: Redactor,
    dirty: Option<Instant>,
    validated: context::Validation,
    deleted: bool,
    revision: u64,
    last_ui_input: Option<Input>,
}

pub enum Stage {
    Begin,
    /// A collaborator returned after the main turn ended; continue without a user message.
    Continue,
    /// Submitted messages joined the turn; the given prompts stay queued.
    Accept(Vec<crate::attachments::Prompt>),
    Completion,
    Tool(String),
    ReadBatch(Vec<String>),
    ToolBatch(Vec<String>),
    ToolBatchResult(crate::json::Value),
    ToolFinished,
    ReadBatchFinished,
    ToolBatchFinished,
    Ready,
    Preserve,
    /// A backend selection must either be saved in full or leave its old state.
    Selection,
}

impl Handle {
    pub fn new(store: Store, document: Document, redactor: Redactor) -> Self {
        Self::with_lease(store, document, redactor, None)
    }

    fn with_lease(
        store: Store,
        document: Document,
        redactor: Redactor,
        lease: Option<Arc<Lease>>,
    ) -> Self {
        Self(Arc::new(Shared {
            live: Mutex::new(Live {
                store,
                document,
                lease,
                redactor,
                dirty: Some(Instant::now()),
                validated: context::Validation::default(),
                deleted: false,
                revision: 0,
                last_ui_input: None,
            }),
            io: Mutex::new(()),
            background_save: AtomicBool::new(false),
            background_result: Mutex::new(None),
            autosave: Arc::default(),
        }))
    }

    pub fn open(store: Store, id: &str, redactor: Redactor) -> Result<(Self, String), String> {
        let lease = store.acquire(id)?;
        let (mut document, damaged) = store.load(id)?;
        document.input = document.input.redacted(&redactor);
        document.messages = document
            .messages
            .iter()
            .map(|message| redactor.value(message))
            .collect();
        document.events = document
            .events
            .iter()
            .map(|event| redactor.value(event))
            .collect();
        document.pending.partial = redactor.text(&document.pending.partial);
        document.context.summary = redactor.text(&document.context.summary);
        document.context.skill_references = redactor.text(&document.context.skill_references);
        document.pending.tool = document.pending.tool.map(|id| redactor.text(&id));
        document.pending.reads = document
            .pending
            .reads
            .into_iter()
            .map(|id| redactor.text(&id))
            .collect();
        document.pending.batch = document
            .pending
            .batch
            .into_iter()
            .map(|id| redactor.text(&id))
            .collect();
        document.pending.completed = document
            .pending
            .completed
            .iter()
            .map(|receipt| redactor.value(receipt))
            .collect();
        document.input.question = document
            .input
            .question
            .map(|draft| draft.redacted(&redactor));
        context::validate(&document)?;
        let interrupted = document.recover();
        context::validate(&document)?;
        let handle = Self::with_lease(store, document, redactor, Some(Arc::new(lease)));
        handle.flush()?;
        let awaiting_question = {
            let saved = handle.snapshot();
            saved.pending.tool.as_ref().is_some_and(|id| {
                saved.events.iter().any(|event| {
                    event.get("type").and_then(crate::json::Value::as_str) == Some("user_question")
                        && event
                            .get("tool_call_id")
                            .and_then(crate::json::Value::as_str)
                            == Some(id)
                })
            })
        };
        Ok((
            handle,
            format!(
                "{}{}",
                if damaged {
                    "Recovered the last valid checkpoint. "
                } else {
                    ""
                },
                if awaiting_question {
                    "Question awaits your answer."
                } else if interrupted {
                    "Interrupted work recovered; ready for a new request."
                } else {
                    "Ready for a new request."
                },
            ),
        ))
    }

    pub fn store(&self) -> Store {
        self.0.live.lock().unwrap().store.clone()
    }
    pub fn snapshot(&self) -> Document {
        self.0.live.lock().unwrap().document.clone()
    }
    pub fn id(&self) -> String {
        self.0.live.lock().unwrap().document.id.clone()
    }

    pub fn active(&self) -> bool {
        self.0.live.lock().unwrap().document.pending.active
    }

    /// Whether this handle holds the session's exclusive writer lease, which
    /// keeps every other conversation out of the session's own storage.
    pub fn leased(&self) -> bool {
        self.0.live.lock().unwrap().lease.is_some()
    }

    pub fn checkpoint(
        &self,
        archive: &Archive,
        compatible_from: usize,
        context: &crate::context::Context,
        routing: &crate::backend::Routing,
        has_model: bool,
        stage: Stage,
    ) -> Result<(), String> {
        if let Stage::ToolBatchResult(receipt) = &stage {
            context::validate_batch_receipt(receipt)?;
        }
        // Disk access is awaited first: a save in progress never holds up the
        // conversation's own history.
        let _io = self.0.io.lock().unwrap();
        let messages = archive.messages.lock().unwrap();
        let events = archive.events.lock().unwrap();
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
        let selection = matches!(stage, Stage::Selection)
            .then(|| (live.document.clone(), live.validated.clone()));
        let preparation = matches!(
            stage,
            Stage::Begin | Stage::Continue | Stage::Accept(_) | Stage::ToolBatchResult(_)
        )
        .then(|| {
            (
                live.document.messages.len(),
                live.document.events.len(),
                live.document.pending.clone(),
                live.validated.clone(),
                live.document.input.clone(),
            )
        });
        let appended = messages
            .iter()
            .skip(live.document.messages.len())
            .map(|message| live.redactor.value(message))
            .collect::<Vec<_>>();
        live.document.messages.extend(appended);
        let appended = events
            .iter()
            .skip(live.document.events.len())
            .map(|event| live.redactor.value(event))
            .collect::<Vec<_>>();
        live.document.events.extend(appended);
        // The redacted tail is copied: the conversation's own history is free
        // again before the save validates and writes.
        drop(events);
        drop(messages);
        live.document.model = has_model.then(|| live.redactor.text(&archive.model));
        live.document.backend = archive.backend.clone();
        live.document.legacy_origin = archive.legacy_origin.clone();
        if let Stage::Accept(queued) = &stage {
            // Accepted messages leave the queue in the same save that adds them.
            live.document.input.queued = queued.clone();
            live.last_ui_input = None;
        }
        live.document.input = live.document.input.redacted(&live.redactor);
        live.document.effort = crate::effort::Effort::parse(&archive.effort)?;
        live.document.compatible_from = compatible_from;
        live.document.context = context.clone();
        live.document.routing = routing.clone();
        match stage {
            Stage::Begin | Stage::Continue => {
                live.document.pending = Pending {
                    active: true,
                    ..Pending::default()
                }
            }
            Stage::Completion => live.document.pending.partial.clear(),
            Stage::Tool(id) => live.document.pending.tool = Some(live.redactor.text(&id)),
            Stage::ReadBatch(ids) => {
                live.document.pending.reads = ids.iter().map(|id| live.redactor.text(id)).collect();
            }
            Stage::ToolBatch(ids) => {
                live.document.pending.batch = ids.iter().map(|id| live.redactor.text(id)).collect();
                live.document.pending.reads = live.document.pending.batch.clone();
                live.document.pending.completed.clear();
            }
            Stage::ToolBatchResult(receipt) => {
                let receipt = live.redactor.value(&receipt);
                live.document.pending.completed.push(receipt);
            }
            Stage::ToolFinished => {
                if live
                    .document
                    .input
                    .question
                    .as_ref()
                    .is_some_and(|draft| Some(&draft.id) == live.document.pending.tool.as_ref())
                {
                    live.document.input.question = None;
                    live.last_ui_input = None;
                }
                live.document.pending.tool = None;
            }
            Stage::ReadBatchFinished => live.document.pending.reads.clear(),
            Stage::ToolBatchFinished => {
                live.document.pending.reads.clear();
                live.document.pending.batch.clear();
                live.document.pending.completed.clear();
            }
            Stage::Ready => live.document.pending = Pending::default(),
            Stage::Accept(_) | Stage::Preserve | Stage::Selection => {}
        }
        live.dirty.get_or_insert_with(Instant::now);
        live.revision = live.revision.wrapping_add(1);
        let revision = live.revision;
        drop(live);
        let result = self.save_locked_io();
        if result.is_err()
            && let Some((document, validated)) = selection
        {
            let mut live = self.0.live.lock().unwrap();
            // UI input can be captured during the save; retain its latest value.
            let input = live.document.input.clone();
            live.document = document;
            live.document.input = input;
            live.validated = validated;
            live.revision = live.revision.wrapping_add(1);
        }
        if result.is_err()
            && let Some((messages, events, pending, validated, input)) = preparation
        {
            let mut live = self.0.live.lock().unwrap();
            live.document.messages.truncate(messages);
            live.document.events.truncate(events);
            live.document.pending = pending;
            live.validated = validated;
            if live.revision == revision {
                live.document.input = input;
            }
            live.last_ui_input = None;
            live.revision = live.revision.wrapping_add(1);
        }
        result
    }

    pub fn input(&self, input: Input) {
        let mut live = self.0.live.lock().unwrap();
        if live.deleted {
            return;
        }
        live.last_ui_input = None;
        let input = input.redacted(&live.redactor);
        if input != live.document.input {
            live.document.input = input;
            live.dirty.get_or_insert_with(Instant::now);
            live.revision = live.revision.wrapping_add(1);
        }
    }

    /// Whether the session already has a generated title.
    pub fn titled(&self) -> bool {
        self.0.live.lock().unwrap().document.title.is_some()
    }

    /// Saves the first generated title; later ones never replace it.
    pub fn name(&self, title: &str) -> Result<(), String> {
        let mut live = self.0.live.lock().unwrap();
        if live.deleted || live.document.title.is_some() {
            return Ok(());
        }
        // Redaction may lengthen it; the saved title keeps the parse limit.
        let title = live.redactor.text(title);
        let title: String = title
            .chars()
            .filter(|c| !c.is_control())
            .take(100)
            .collect();
        let title = title.trim();
        if title.is_empty() {
            return Ok(());
        }
        live.document.title = Some(title.into());
        live.dirty.get_or_insert_with(Instant::now);
        live.revision = live.revision.wrapping_add(1);
        drop(live);
        self.flush()
    }

    /// Capture composer state without copying the conversation or replacing
    /// attachments staged by the line-based chat.
    pub fn capture_input(&self, mut input: Input) {
        let mut live = self.0.live.lock().unwrap();
        if live.deleted || live.last_ui_input.as_ref() == Some(&input) {
            return;
        }
        input.staged.clone_from(&live.document.input.staged);
        let redacted = input.redacted(&live.redactor);
        input.staged.clear();
        live.last_ui_input = Some(input);
        if redacted != live.document.input {
            live.document.input = redacted;
            live.dirty.get_or_insert_with(Instant::now);
            live.revision = live.revision.wrapping_add(1);
        }
    }

    pub fn partial(&self, text: &str) -> Result<(), String> {
        let mut live = self.0.live.lock().unwrap();
        if live.deleted {
            return Err("This session was deleted; start a new conversation".into());
        }
        let first = live.document.pending.partial.is_empty() && !text.is_empty();
        live.document.pending.partial = live.redactor.text(text);
        live.dirty.get_or_insert_with(Instant::now);
        live.revision = live.revision.wrapping_add(1);
        drop(live);
        // Keep the first visible prefix durable, then let periodic autosaves
        // run without blocking streaming on journal fsync.
        if first {
            self.flush()?;
        } else {
            self.flush_due_background()?;
        }
        Ok(())
    }

    /// Poll the completed save and schedule a due save without waiting for disk I/O.
    pub fn flush_due_background(&self) -> Result<bool, String> {
        if let Some(result) = self.0.background_result.lock().unwrap().take() {
            let dirty = self.0.live.lock().unwrap().dirty.is_some();
            if let Err(error) = result
                && dirty
            {
                return Err(error);
            }
            return Ok(!dirty);
        }
        if self.0.background_save.load(Ordering::Acquire) {
            return Ok(false);
        }
        let due = self
            .0
            .live
            .lock()
            .unwrap()
            .dirty
            .is_some_and(|since| since.elapsed() >= SAVE_INTERVAL);
        if !due || self.0.background_save.swap(true, Ordering::AcqRel) {
            return Ok(false);
        }
        if let Err(error) = self.0.autosave.request(&self.0) {
            self.0.background_save.store(false, Ordering::Release);
            return Err(error);
        }
        Ok(false)
    }

    pub fn flush(&self) -> Result<(), String> {
        let _io = self.0.io.lock().unwrap();
        self.save_locked_io()
    }

    pub(crate) fn delete(
        &self,
        live_attachments: &std::collections::BTreeSet<String>,
        before_remove: impl FnOnce(),
    ) -> Result<super::DeleteReport, String> {
        let _io = self.0.io.lock().unwrap();
        let mut live = self.0.live.lock().unwrap();
        if live.deleted || live.document.pending.active {
            return Err("Session deletion requires an open, ready conversation".into());
        }
        live.store.deletion_scope()?;
        if live.lease.is_none() {
            live.lease = Some(Arc::new(live.store.acquire(&live.document.id)?));
        }
        let removal = live.store.removal(&live.document)?;
        before_remove();
        let report = removal.remove_with_live(live_attachments)?;
        // Every clone must stop saving before releasing the exclusive lease.
        live.deleted = true;
        live.dirty = None;
        live.lease = None;
        Ok(report)
    }
}

#[cfg(test)]
mod collaboration_tests;
#[cfg(test)]
mod tests;
