//! Prompts submitted while the agent works, shared by the interface and the
//! agent. Ordinary messages join the active turn before its next model
//! request; local commands stay here until the interface runs them while idle.
//! One lock orders every change, so a prompt is either accepted into the
//! conversation or still pending, never both.

use crate::attachments::Prompt;
use crate::cancel::Cancellation;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

/// Pending prompts a busy interface can hold at once.
pub const LIMIT: usize = 8;

#[derive(Clone, Default)]
pub struct Inbox(Arc<Shared>);

#[derive(Default)]
struct Shared {
    entries: Mutex<Entries>,
    changed: Condvar,
}

#[derive(Default)]
pub struct Entries {
    items: VecDeque<(u64, Prompt)>,
    next: u64,
    /// Entries below this identifier are in the interface's saved input,
    /// which no longer holds them as composer text or paused drafts.
    recorded: u64,
    /// The entry being edited and its unsaved revision. It and every later
    /// entry wait, which keeps ordinary messages in submission order.
    held: Option<(u64, Option<Prompt>)>,
    /// Accepted prompts the interface has not displayed yet.
    unacknowledged: VecDeque<Prompt>,
}

/// What a turn without further tool calls does next.
pub enum Boundary {
    Accepted(Vec<Prompt>),
    Held,
    Closed,
}

/// Local commands start with a slash and have no attachments.
pub fn ordinary(prompt: &Prompt) -> bool {
    !prompt.attachments.is_empty() || !prompt.text.trim_start().starts_with('/')
}

impl Inbox {
    /// Reads or revises entries without waking a waiting turn.
    pub fn with<R>(&self, read: impl FnOnce(&mut Entries) -> R) -> R {
        read(&mut self.0.entries.lock().unwrap())
    }

    /// Changes entries and wakes a turn waiting for an edit.
    pub fn update<R>(&self, change: impl FnOnce(&mut Entries) -> R) -> R {
        let mut entries = self.0.entries.lock().unwrap();
        let result = change(&mut entries);
        self.0.changed.notify_all();
        result
    }

    /// Wakes a waiting turn so it can observe a cancellation request.
    pub fn wake(&self) {
        self.update(|_| ());
    }

    /// Accepts deliverable ordinary messages in submission order. `accept`
    /// receives them and the entries that remain pending; if it fails, every
    /// message stays pending.
    pub fn accept(
        &self,
        accept: impl FnOnce(&[Prompt], Vec<Prompt>) -> Result<(), String>,
    ) -> Result<Vec<Prompt>, String> {
        let mut entries = self.0.entries.lock().unwrap();
        entries.accept(accept)
    }

    /// Accepts deliverable messages or, when there are none, decides whether
    /// the turn closes or an edit still holds messages back. Messages pending
    /// at this point never wait for a later turn.
    pub fn close(
        &self,
        accept: impl FnOnce(&[Prompt], Vec<Prompt>) -> Result<(), String>,
    ) -> Result<Boundary, String> {
        let mut entries = self.0.entries.lock().unwrap();
        let accepted = entries.accept(accept)?;
        Ok(if !accepted.is_empty() {
            Boundary::Accepted(accepted)
        } else if entries.held_back() {
            Boundary::Held
        } else {
            Boundary::Closed
        })
    }

    /// Blocks until an edit stops holding messages back or work is cancelled.
    pub fn wait(&self, cancellation: &Cancellation) -> Result<(), String> {
        let mut entries = self.0.entries.lock().unwrap();
        loop {
            if cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            if !entries.held_back() || entries.ready() {
                return Ok(());
            }
            entries = self.0.changed.wait(entries).unwrap();
        }
    }
}

impl Entries {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn prompts(&self) -> impl Iterator<Item = (u64, &Prompt)> {
        self.items.iter().map(|(id, prompt)| (*id, prompt))
    }

    /// Whether an ordinary message can join the conversation now.
    pub fn ready(&self) -> bool {
        !self.deliverable().is_empty()
    }

    pub fn position(&self, id: u64) -> Option<usize> {
        self.items.iter().position(|(entry, _)| *entry == id)
    }

    pub fn push(&mut self, prompt: Prompt) -> Result<u64, String> {
        if self.items.len() >= LIMIT {
            return Err(format!(
                "Queue is full ({LIMIT} messages). Your draft was kept."
            ));
        }
        let id = self.next;
        self.next += 1;
        self.items.push_back((id, prompt));
        Ok(id)
    }

    pub fn pop_front(&mut self) -> Option<Prompt> {
        if self.held.is_some() {
            return None;
        }
        self.items.pop_front().map(|(_, prompt)| prompt)
    }

    pub fn remove(&mut self, id: u64) -> Option<Prompt> {
        if self.held.as_ref().is_some_and(|(held, _)| *held == id) {
            return None;
        }
        let position = self.position(id)?;
        self.items.remove(position).map(|(_, prompt)| prompt)
    }

    /// Holds an entry for editing; `None` means it was already accepted.
    pub fn hold(&mut self, id: u64) -> Option<Prompt> {
        if self.held.is_some() {
            return None;
        }
        let prompt = self.items.get(self.position(id)?)?.1.clone();
        self.held = Some((id, None));
        Some(prompt)
    }

    /// Records the unsaved text of the held entry for session snapshots.
    pub fn revise(&mut self, prompt: Prompt) {
        if let Some((_, revision)) = &mut self.held {
            *revision = Some(prompt);
        }
    }

    /// Ends an edit, replacing the entry when `prompt` is given.
    pub fn release(&mut self, prompt: Option<Prompt>) {
        if let Some((id, _)) = self.held.take()
            && let Some(prompt) = prompt
            && let Some(position) = self.position(id)
        {
            self.items[position].1 = prompt;
        }
    }

    /// Removes every entry, ending any edit, and returns the held position.
    pub fn drain(&mut self) -> (Vec<Prompt>, Option<usize>) {
        let held = self.held.take().and_then(|(id, _)| self.position(id));
        let prompts = self.items.drain(..).map(|(_, prompt)| prompt).collect();
        (prompts, held)
    }

    /// Pending prompts for the interface's saved input. Only recorded
    /// entries can join a turn, so an accepted message never remains in
    /// another saved form.
    pub fn record(&mut self) -> Vec<Prompt> {
        self.recorded = self.next;
        self.snapshot()
    }

    /// Pending prompts as a session saves them, including an unsaved edit.
    fn snapshot(&self) -> Vec<Prompt> {
        self.items
            .iter()
            .map(|(id, prompt)| match &self.held {
                Some((held, Some(revision))) if held == id => revision.clone(),
                _ => prompt.clone(),
            })
            .collect()
    }

    /// Marks the oldest accepted prompts as displayed.
    pub fn acknowledge(&mut self, count: usize) {
        let count = count.min(self.unacknowledged.len());
        self.unacknowledged.drain(..count);
    }

    /// Accepted prompts whose notification never reached the interface.
    pub fn unacknowledged(&mut self) -> Vec<Prompt> {
        self.unacknowledged.drain(..).collect()
    }

    fn accept(
        &mut self,
        accept: impl FnOnce(&[Prompt], Vec<Prompt>) -> Result<(), String>,
    ) -> Result<Vec<Prompt>, String> {
        let ids = self.deliverable();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let prompts: Vec<_> = self
            .prompts()
            .filter(|(id, _)| ids.contains(id))
            .map(|(_, prompt)| prompt.clone())
            .collect();
        let remaining = self
            .items
            .iter()
            .zip(self.snapshot())
            .filter(|((id, _), _)| !ids.contains(id))
            .map(|(_, prompt)| prompt)
            .collect();
        accept(&prompts, remaining)?;
        self.items.retain(|(id, _)| !ids.contains(id));
        self.unacknowledged.extend(prompts.iter().cloned());
        Ok(prompts)
    }

    /// Recorded ordinary messages ahead of any held entry.
    fn deliverable(&self) -> Vec<u64> {
        let held = self.held.as_ref().map(|(id, _)| *id);
        self.items
            .iter()
            .take_while(|(id, _)| Some(*id) != held && *id < self.recorded)
            .filter(|(_, prompt)| ordinary(prompt))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Whether an edit holds back an ordinary message.
    fn held_back(&self) -> bool {
        let Some((held, _)) = &self.held else {
            return false;
        };
        self.items
            .iter()
            .skip_while(|(id, _)| id != held)
            .any(|(_, prompt)| ordinary(prompt))
    }
}

#[cfg(test)]
mod tests;
