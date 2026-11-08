//! Exact native records: the first and usually only reduction a conversation needs.

use crate::{agent::Agent, context, json::Value};

impl Agent {
    fn write_premium(&self) -> f64 {
        self.client.cache_write_premium(self.cache_retention())
    }

    pub(super) fn digest_window(&self, messages: &[Value], budget: usize) -> usize {
        if !self.records {
            return budget;
        }
        self.context
            .digest_budget(messages, budget, self.write_premium())
    }

    /// Where a new turn's requests begin, when nothing has answered them yet and
    /// the turn before finished. A message sent while tools ran is not a new turn:
    /// the results before it may be unread.
    pub(super) fn turn_start(&self, messages: &[Value]) -> Option<usize> {
        if !self.records {
            return None;
        }
        let role = |index: &usize| messages[*index].get("role").and_then(Value::as_str);
        let mut visible = (self.context.from..messages.len())
            .rev()
            .filter(|index| !self.context.returned.contains(index));
        let mut start = visible.next().filter(|index| role(index) == Some("user"))?;
        for index in visible {
            match role(&index) {
                Some("user") => start = index,
                Some("assistant") if messages[index].get("tool_calls").is_none() => break,
                _ => return None,
            }
        }
        Some(start)
    }

    /// The context after a digest, when that is worthwhile. A new turn keeps only
    /// its requests, since nothing recent is under way. Work under way keeps its
    /// latest exchange whole, whose results may be unread, and earlier ones while
    /// they fit. Oversized results shrink to previews only when records fall short.
    pub(super) fn prepared_digest(
        &self,
        environment: &crate::agent::environment::Environment,
        messages: &[Value],
        estimate: usize,
        budget: usize,
        forced: bool,
        turn: Option<usize>,
    ) -> Option<context::Context> {
        let window = self.digest_window(messages, budget);
        let latest = self.latest_exchange(messages);
        let end = turn.unwrap_or_else(|| self.recent_boundary(messages, window / 4).min(latest));
        let gain = turn.map_or(window / 4, |_| {
            context::digest::turn_gain(budget, self.write_premium())
        });
        let after =
            |context: &context::Context| self.context_estimate_for(context, environment, messages);
        if self.records && end > self.context.from {
            let mut records = self.context.clone();
            records.clear_measurement();
            records.states.retain(|state| state.before >= end);
            records.push_digest(end, self.record_notes(end));
            records.roll_up(messages, budget);
            let size = after(&records);
            let saved = estimate.saturating_sub(size);
            // At the budget any saving beats a summary, which costs a model request.
            if size < budget && saved > 0 && (forced || estimate >= budget || saved >= gain) {
                return Some(records);
            }
        }
        if !forced && estimate < budget {
            return None;
        }
        // Records fall short: the tool payloads themselves must shrink.
        let mut previews = self.context.clone();
        previews.clear_measurement();
        previews.preview_until = messages.len();
        previews.preview_limit = (budget / 12).clamp(512, 2048);
        let size = after(&previews);
        let smaller = self.context_bytes(&self.projected_context(&previews, messages))
            < self.context_bytes(&self.projected_context(&self.context, messages));
        (smaller && size < budget).then_some(previews)
    }

    /// Where the latest exchange starts. Results nothing has answered stay with the
    /// call that asked for them, even when a message was sent after them.
    fn latest_exchange(&self, messages: &[Value]) -> usize {
        let role = |index: &usize| messages[*index].get("role").and_then(Value::as_str);
        let mut visible = (self.context.from..messages.len())
            .rev()
            .filter(|index| !self.context.returned.contains(index));
        let unanswered = visible
            .clone()
            .find(|index| role(index) == Some("assistant"))
            .filter(|index| {
                messages[*index]
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .is_some_and(|calls| !calls.is_empty())
            });
        unanswered
            .or_else(|| visible.find(|index| role(index) != Some("tool")))
            .unwrap_or(self.context.from)
    }

    /// What a digest must carry beyond its messages: outcomes reported only in
    /// runtime state, and skills that were read, now out of view.
    fn record_notes(&self, end: usize) -> String {
        let mut notes = self
            .context
            .states
            .iter()
            .filter(|state| state.before < end)
            .filter_map(|state| crate::agent::projection::process_section(&state.text))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let skills = self.redact(&self.skill_checkpoint_instructions());
        if !skills.is_empty()
            && !self
                .context
                .digests
                .iter()
                .any(|epoch| epoch.notes.contains(&skills))
        {
            notes.push(skills);
        }
        notes.join("\n")
    }
}

#[cfg(not(test))]
pub(in crate::agent) fn records_by_default() -> bool {
    true
}

#[cfg(test)]
thread_local! {
    static RECORDS_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(in crate::agent) fn records_by_default() -> bool {
    !RECORDS_OFF.get()
}

/// Agents this test creates start with previews and summaries as their first tier.
#[cfg(test)]
pub(in crate::agent) fn test_fallback_tiers() {
    RECORDS_OFF.set(true);
}
