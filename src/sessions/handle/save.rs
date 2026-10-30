//! One save: the checkpoint is copied out of the live document, then written
//! with the `live` lock released.

use super::{super::Document, Handle};
use crate::{json::Value, sessions::context, sessions::journal::Tail, sessions::now};
use std::{sync::Arc, time::Instant};

/// A checkpoint copied out of the live document.
struct Record {
    document: Document,
    title: String,
    messages_from: usize,
    messages: Vec<Value>,
    events_from: usize,
    events: Vec<Value>,
}

impl Handle {
    /// Copies under the `live` lock only the state and the history the
    /// journal lacks; the write and its sync run after the lock is released.
    /// The caller holds the `io` lock.
    pub(super) fn save_locked_io(&self) -> Result<(), String> {
        let (record, store, lease, revision, validated) = {
            let mut live = self.0.live.lock().unwrap();
            if live.deleted || live.dirty.is_none() {
                return Ok(());
            }
            if !live.document.meaningful() && live.lease.is_none() {
                live.dirty = None;
                return Ok(());
            }
            // A failed attempt remains dirty, with a bounded interval before retry.
            live.dirty = Some(Instant::now());
            let mut validated = live.validated.clone();
            context::validate_from(&live.document, &mut validated)?;
            // The io lock is held, so the journal cannot move meanwhile.
            let (messages, events) = live
                .lease
                .as_ref()
                .and_then(|lease| lease.history())
                .filter(|&(messages, events)| {
                    messages <= live.document.messages.len() && events <= live.document.events.len()
                })
                .unwrap_or((0, 0));
            let mut document = live.document.without_history();
            document.updated = now();
            let record = Record {
                title: live.document.title(),
                messages_from: messages,
                messages: live.document.messages[messages..].to_vec(),
                events_from: events,
                events: live.document.events[events..].to_vec(),
                document,
            };
            (
                record,
                live.store.clone(),
                live.lease.clone(),
                live.revision,
                validated,
            )
        };
        let document = &record.document;
        let lease = match lease {
            Some(lease) => lease,
            None => {
                let lease = Arc::new(store.create(&document.id)?);
                self.0.live.lock().unwrap().lease = Some(lease.clone());
                lease
            }
        };
        let result = lease.save_tail(
            &Tail {
                document,
                messages_from: record.messages_from,
                messages: &record.messages,
                events_from: record.events_from,
                events: &record.events,
            },
            &record.title,
        );
        if result.is_ok() {
            let mut live = self.0.live.lock().unwrap();
            live.validated = validated;
            live.document.updated = document.updated;
            if live.revision == revision {
                live.dirty = None;
            }
        }
        result
    }
}
