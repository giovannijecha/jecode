use super::{Document, Input, Pending, Store, context, now, storage::Lease};
use crate::{export::Archive, redact::Redactor};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SAVE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct Handle(Arc<Mutex<Live>>);

struct Live {
    store: Store,
    document: Document,
    lease: Option<Lease>,
    redactor: Redactor,
    dirty: Option<Instant>,
    validated: context::Validation,
    deleted: bool,
}

pub enum Stage {
    Begin,
    Completion,
    Tool(String),
    ToolFinished,
    Ready,
    Preserve,
}

impl Handle {
    pub fn new(store: Store, document: Document, redactor: Redactor) -> Self {
        Self(Arc::new(Mutex::new(Live {
            store,
            document,
            lease: None,
            redactor,
            dirty: Some(Instant::now()),
            validated: context::Validation::default(),
            deleted: false,
        })))
    }

    pub fn open(store: Store, id: &str, redactor: Redactor) -> Result<(Self, String), String> {
        let lease = store.acquire(id)?;
        let (document, backup) = store.load(id)?;
        if backup {
            store.preserve_legacy_damage(id)?;
        }
        let mut document = document;
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
        document.pending.tool = document.pending.tool.map(|id| redactor.text(&id));
        context::validate(&document)?;
        let interrupted = document.recover();
        context::validate(&document)?;
        let handle = Self(Arc::new(Mutex::new(Live {
            store,
            document,
            lease: Some(lease),
            redactor,
            dirty: Some(Instant::now()),
            validated: context::Validation::default(),
            deleted: false,
        })));
        handle.flush()?;
        Ok((
            handle,
            format!(
                "{}{}",
                if backup {
                    "Recovered the last valid checkpoint. "
                } else {
                    ""
                },
                if interrupted {
                    "Interrupted work recovered; ready for a new request."
                } else {
                    "Ready for a new request."
                },
            ),
        ))
    }

    pub fn store(&self) -> Store {
        self.0.lock().unwrap().store.clone()
    }
    pub fn snapshot(&self) -> Document {
        self.0.lock().unwrap().document.clone()
    }
    pub fn id(&self) -> String {
        self.0.lock().unwrap().document.id.clone()
    }

    pub fn active(&self) -> bool {
        self.0.lock().unwrap().document.pending.active
    }

    pub fn checkpoint(
        &self,
        archive: &Archive,
        compatible_from: usize,
        context: &crate::context::Context,
        stage: Stage,
    ) -> Result<(), String> {
        let messages = archive.messages.lock().unwrap();
        let events = archive.events.lock().unwrap();
        let mut live = self.0.lock().unwrap();
        if live.deleted {
            return Err("This session was deleted; start a new conversation".into());
        }
        live.redactor.include(archive.redactor.clone());
        if messages.len() < live.document.messages.len()
            || events.len() < live.document.events.len()
        {
            return Err("Session history cannot shrink; start a new conversation instead".into());
        }
        let preparation = matches!(stage, Stage::Begin).then(|| {
            (
                live.document.messages.len(),
                live.document.events.len(),
                live.document.pending.clone(),
                live.validated.clone(),
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
        live.document.model = live.redactor.text(&archive.model);
        live.document.input = live.document.input.redacted(&live.redactor);
        live.document.effort = crate::effort::Effort::parse(&archive.effort)?;
        live.document.compatible_from = compatible_from;
        live.document.context = context.clone();
        match stage {
            Stage::Begin => {
                live.document.pending = Pending {
                    active: true,
                    ..Pending::default()
                }
            }
            Stage::Completion => live.document.pending.partial.clear(),
            Stage::Tool(id) => live.document.pending.tool = Some(live.redactor.text(&id)),
            Stage::ToolFinished => live.document.pending.tool = None,
            Stage::Ready => live.document.pending = Pending::default(),
            Stage::Preserve => {}
        }
        live.dirty.get_or_insert_with(Instant::now);
        let result = live.save();
        if result.is_err()
            && let Some((messages, events, pending, validated)) = preparation
        {
            live.document.messages.truncate(messages);
            live.document.events.truncate(events);
            live.document.pending = pending;
            live.validated = validated;
        }
        result
    }

    pub fn input(&self, input: Input) {
        let mut live = self.0.lock().unwrap();
        if live.deleted {
            return;
        }
        let input = input.redacted(&live.redactor);
        if input != live.document.input {
            live.document.input = input;
            live.dirty.get_or_insert_with(Instant::now);
        }
    }

    pub fn partial(&self, text: &str) -> Result<(), String> {
        let mut live = self.0.lock().unwrap();
        if live.deleted {
            return Err("This session was deleted; start a new conversation".into());
        }
        let first = live.document.pending.partial.is_empty() && !text.is_empty();
        live.document.pending.partial = live.redactor.text(text);
        live.dirty.get_or_insert_with(Instant::now);
        if first
            || live
                .dirty
                .is_some_and(|since| since.elapsed() >= SAVE_INTERVAL)
        {
            live.save()?;
        }
        Ok(())
    }

    pub fn flush_due(&self) -> Result<bool, String> {
        let mut live = self.0.lock().unwrap();
        if live
            .dirty
            .is_some_and(|since| since.elapsed() >= SAVE_INTERVAL)
        {
            live.save()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn flush(&self) -> Result<(), String> {
        self.0.lock().unwrap().save()
    }

    pub(crate) fn delete(
        &self,
        live_attachments: &std::collections::BTreeSet<String>,
    ) -> Result<super::DeleteReport, String> {
        let mut live = self.0.lock().unwrap();
        if live.deleted || live.document.pending.active {
            return Err("Session deletion requires an open, ready conversation".into());
        }
        live.store.deletion_scope()?;
        if live.lease.is_none() {
            live.lease = Some(live.store.acquire(&live.document.id)?);
        }
        let report = live
            .store
            .removal(&live.document)?
            .remove_with_live(live_attachments)?;
        // Every clone must stop saving before releasing the exclusive lease.
        live.deleted = true;
        live.dirty = None;
        live.lease = None;
        Ok(report)
    }
}

impl Live {
    fn save(&mut self) -> Result<(), String> {
        if self.deleted || self.dirty.is_none() {
            return Ok(());
        }
        if !self.document.meaningful() && self.lease.is_none() {
            self.dirty = None;
            return Ok(());
        }
        // A failed attempt remains dirty, with a bounded interval before retry.
        self.dirty = Some(Instant::now());
        context::validate_from(&self.document, &mut self.validated)?;
        self.document.updated = now();
        if self.lease.is_none() {
            self.lease = Some(self.store.create(&self.document.id)?);
        }
        self.lease.as_ref().unwrap().save(&self.document)?;
        self.dirty = None;
        Ok(())
    }
}
