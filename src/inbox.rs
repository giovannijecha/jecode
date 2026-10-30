//! Prompts submitted while the agent works, shared by the interface and the
//! agent. Ordinary messages join the active turn before its next model
//! request; local commands stay here until the interface runs them while idle.
//! One lock orders every change, so a prompt is either accepted into the
//! conversation or still pending, never both.

use crate::attachments::Prompt;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Pending prompts an inbox can hold at once.
pub const LIMIT: usize = 8;

#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Entries>>);

#[derive(Default)]
pub struct Entries {
    items: VecDeque<(u64, Prompt)>,
    next: u64,
    /// Entries below this identifier are in the interface's saved input,
    /// which no longer holds them as composer text or a paused draft.
    recorded: u64,
    /// Accepted prompts the interface has not displayed yet.
    unacknowledged: VecDeque<Prompt>,
}

/// Local commands start with a slash and have no attachments.
pub fn ordinary(prompt: &Prompt) -> bool {
    !prompt.attachments.is_empty() || !prompt.text.trim_start().starts_with('/')
}

impl Inbox {
    /// Reads or changes entries under the lock.
    pub fn with<R>(&self, read: impl FnOnce(&mut Entries) -> R) -> R {
        read(&mut self.0.lock().unwrap())
    }

    /// Accepts deliverable ordinary messages in submission order. `accept`
    /// receives them and the entries that remain pending; if it fails, every
    /// message stays pending.
    pub fn accept(
        &self,
        accept: impl FnOnce(&[Prompt], Vec<Prompt>) -> Result<(), String>,
    ) -> Result<Vec<Prompt>, String> {
        self.with(|entries| entries.accept(accept))
    }
}

impl Entries {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn prompts(&self) -> impl Iterator<Item = &Prompt> {
        self.items.iter().map(|(_, prompt)| prompt)
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
        self.items.pop_front().map(|(_, prompt)| prompt)
    }

    /// Removes every entry.
    pub fn drain(&mut self) -> Vec<Prompt> {
        self.items.drain(..).map(|(_, prompt)| prompt).collect()
    }

    /// Pending prompts for the interface's saved input. Only recorded
    /// entries can join a turn, so an accepted message never remains in
    /// another saved form.
    pub fn record(&mut self) -> Vec<Prompt> {
        self.recorded = self.next;
        self.prompts().cloned().collect()
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
        let (accepted, remaining): (Vec<_>, Vec<_>) =
            self.items.iter().partition(|(id, _)| ids.contains(id));
        let prompts: Vec<_> = accepted
            .into_iter()
            .map(|(_, prompt)| prompt.clone())
            .collect();
        let remaining = remaining
            .into_iter()
            .map(|(_, prompt)| prompt.clone())
            .collect();
        accept(&prompts, remaining)?;
        self.items.retain(|(id, _)| !ids.contains(id));
        self.unacknowledged.extend(prompts.iter().cloned());
        Ok(prompts)
    }

    /// Recorded ordinary messages.
    fn deliverable(&self) -> Vec<u64> {
        self.items
            .iter()
            .take_while(|(id, _)| *id < self.recorded)
            .filter(|(_, prompt)| ordinary(prompt))
            .map(|(id, _)| *id)
            .collect()
    }
}

#[cfg(test)]
mod tests;
