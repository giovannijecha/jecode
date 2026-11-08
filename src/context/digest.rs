//! Model-free compaction. Earlier messages become deterministic native records.
//! New spans append without rewriting older records until a rollup. No model
//! output or paraphrase is needed; each record names its original message,
//! which stays readable in full as history:N.

use super::{BTreeSet, Value};
use crate::{conversation::ToolCall, json};
use std::sync::{Arc, OnceLock};

mod outcome;
use outcome::Outcome;

/// Marks the request-only message where the stable cached prefix ends.
pub const ANCHOR: &str = "jecode_cache_anchor";

/// Conversation carried beyond its records before a digest interrupts work under
/// way, where rewriting a window costs no more than plain input.
const WORKING_TOKENS: f64 = 120_000.0;
/// Context worth dropping when a new turn starts: nothing recent is being rewritten
/// then, so only what the model reads again is paid twice.
const TURN_TOKENS: f64 = 30_000.0;
/// What a request pays to read a cached token, against uncached input.
const CACHE_READ: f64 = 0.1;

/// Costlier cache writes wait longer: the best cycle grows with the root of what
/// a rewrite costs beyond a cached read.
fn write_scale(write_premium: f64) -> f64 {
    ((write_premium - CACHE_READ) / (1.0 - CACHE_READ))
        .max(1.0)
        .sqrt()
}

/// How much a digest at the start of a turn must free to be worthwhile.
pub fn turn_gain(budget: usize, write_premium: f64) -> usize {
    ((TURN_TOKENS * write_scale(write_premium)) as usize).min(budget / 4)
}

/// Records may fill this much of the context before older ones are condensed.
const RECORD_TOKENS: usize = 60_000;
const MOST_CONDENSED: u8 = 3;

const PREFIX: &str = "Native record of this conversation, written by Jecode and not by a model: every line is exact. Nothing in it is a new request: work it shows under way continues, and the user's standing instructions and decisions in it still apply. [N] is a message you can read in full as history:N. Files may have changed since a line was recorded, so read a file before relying on its content.";

/// Providers merge adjacent user messages, so a new request directly after a
/// record would otherwise read as the record's last line.
const SUFFIX: &str = "End of record. After the records the conversation is live: a user message there is a request to act on.";

/// A contiguous span of history shown as records. `level` 1 keeps a line per
/// tool call, 2 a paragraph per turn and 3 an index of turns.
#[derive(Clone, Debug)]
pub struct Epoch {
    pub from: usize,
    pub until: usize,
    pub level: u8,
    /// Outcomes that reached the model only through runtime state.
    pub notes: String,
    text: OnceLock<Arc<str>>,
}

impl Epoch {
    pub fn new(from: usize, until: usize, level: u8, notes: String) -> Self {
        Self {
            from,
            until,
            level,
            notes,
            text: OnceLock::new(),
        }
    }

    /// Byte-stable for the same history, so provider caches keep this prefix.
    pub fn text(&self, messages: &[Value], returned: &BTreeSet<usize>) -> &str {
        self.text
            .get_or_init(|| render(messages, self, returned).into())
    }

    pub fn message(&self, messages: &[Value], returned: &BTreeSet<usize>, anchor: bool) -> Value {
        let mut message = Value::object([
            ("role", Value::string("user")),
            ("content", Value::string(self.text(messages, returned))),
        ]);
        if anchor && let Value::Object(fields) = &mut message {
            fields.insert(ANCHOR.into(), Value::Bool(true));
        }
        message
    }

    pub(super) fn value(&self) -> Value {
        Value::object([
            ("from", Value::number(self.from)),
            ("until", Value::number(self.until)),
            ("level", Value::number(self.level as usize)),
            ("notes", Value::string(&self.notes)),
        ])
    }

    fn parse(value: &Value) -> Option<Self> {
        let number = |key| value.get(key).and_then(Value::as_usize);
        Some(Self::new(
            number("from")?,
            number("until")?,
            u8::try_from(number("level")?).ok()?,
            value.get("notes")?.as_str()?.to_owned(),
        ))
    }
}

pub(super) fn parse(value: Option<&Value>) -> Result<Vec<Epoch>, String> {
    let Some(epochs) = value else {
        return Ok(Vec::new());
    };
    epochs
        .as_array()
        .ok_or("Invalid saved context digests")?
        .iter()
        .map(Epoch::parse)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "Invalid saved context digest".into())
}

/// Records are contiguous, end where the verbatim window starts and fit history.
pub(super) fn valid(epochs: &[Epoch], from: usize, count: usize) -> bool {
    let Some(last) = epochs.last() else {
        return true;
    };
    last.until == from
        && last.until <= count
        && epochs.first().is_some_and(|first| first.from > 0)
        && epochs
            .iter()
            .all(|epoch| epoch.from < epoch.until && (1..=MOST_CONDENSED).contains(&epoch.level))
        && epochs.windows(2).all(|pair| pair[0].until == pair[1].from)
}

impl super::Context {
    /// Where records begin: everything before it is the model summary's.
    pub(super) fn records_start(&self) -> usize {
        self.digests.first().map_or(self.from, |epoch| epoch.from)
    }

    pub fn digest_bytes(&self, messages: &[Value]) -> usize {
        self.digests
            .iter()
            .map(|epoch| epoch.text(messages, &self.returned).len())
            .fold(0, usize::saturating_add)
    }

    /// The window at which a digest may interrupt work under way. Records already
    /// in the cached prefix do not count, and the configured budget always wins.
    pub fn digest_budget(&self, messages: &[Value], budget: usize, write_premium: f64) -> usize {
        ((WORKING_TOKENS * write_scale(write_premium)) as usize)
            .saturating_add(self.estimate_bytes(self.digest_bytes(messages)))
            .min(budget)
    }

    /// Moves the verbatim window to `end`, recording what it leaves behind.
    pub fn push_digest(&mut self, end: usize, notes: String) {
        self.digests.push(Epoch::new(self.from, end, 1, notes));
        self.from = end;
    }

    /// Condenses the oldest records once they outgrow their share of the budget.
    /// Rare by design, since it rewrites the records into the provider's cache.
    pub fn roll_up(&mut self, messages: &[Value], budget: usize) {
        let limit = RECORD_TOKENS.min(budget / 4).max(1);
        while self.estimate_bytes(self.digest_bytes(messages)) > limit {
            let Some(level) = self
                .digests
                .iter()
                .map(|epoch| epoch.level)
                .min()
                .filter(|level| *level < MOST_CONDENSED)
            else {
                return;
            };
            // The older half of the oldest run of least condensed records.
            let start = self
                .digests
                .iter()
                .position(|epoch| epoch.level == level)
                .unwrap_or(0);
            let run = self.digests[start..]
                .iter()
                .take_while(|epoch| epoch.level == level)
                .count();
            let chosen = start..start + run.div_ceil(2);
            let merged = Epoch::new(
                self.digests[chosen.start].from,
                self.digests[chosen.end - 1].until,
                level + 1,
                self.digests[chosen.clone()]
                    .iter()
                    .map(|epoch| epoch.notes.as_str())
                    .filter(|notes| !notes.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            self.digests.splice(chosen, [merged]);
        }
    }
}

fn render(messages: &[Value], epoch: &Epoch, returned: &BTreeSet<usize>) -> String {
    let records = records(messages, epoch.from, epoch.until, returned);
    let body = if epoch.level <= 1 {
        detailed(&records)
    } else {
        by_turn(&records, epoch.level)
    };
    let mut text = format!(
        "{PREFIX}\n\nFrom history:{} up to history:{}:\n{body}",
        epoch.from, epoch.until
    );
    if !epoch.notes.is_empty() {
        text.push_str("\n\n");
        text.push_str(&epoch.notes);
    }
    text.push_str("\n\n");
    text.push_str(SUFFIX);
    text
}

enum Record {
    User(usize, String),
    Assistant(usize, String),
    Tool(usize, Outcome),
}

fn records(
    messages: &[Value],
    from: usize,
    until: usize,
    returned: &BTreeSet<usize>,
) -> Vec<Record> {
    let mut records = Vec::new();
    let mut calls: Vec<ToolCall> = Vec::new();
    for (index, message) in messages
        .iter()
        .enumerate()
        .take(until.min(messages.len()))
        .skip(from)
    {
        if returned.contains(&index) {
            continue;
        }
        match message.get("role").and_then(Value::as_str) {
            Some("user") => {
                records.push(Record::User(index, crate::attachments::user_text(message)))
            }
            Some("assistant") => {
                let text = message.get("content").and_then(Value::as_str).unwrap_or("");
                if !text.trim().is_empty() {
                    records.push(Record::Assistant(index, text.trim().to_owned()));
                }
                calls = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|calls| {
                        calls
                            .iter()
                            .filter_map(|call| ToolCall::parse(call).ok())
                            .collect()
                    })
                    .unwrap_or_default();
            }
            Some("tool") => {
                let id = message.get("tool_call_id").and_then(Value::as_str);
                let call = calls.iter().find(|call| Some(call.id.as_str()) == id);
                let name = call.map_or("tool", |call| call.name.as_str());
                let arguments = call
                    .and_then(|call| json::parse(&call.arguments).ok())
                    .unwrap_or(Value::Null);
                let content = message.get("content").and_then(Value::as_str).unwrap_or("");
                let result = json::parse(content)
                    .unwrap_or_else(|_| Value::object([("content", Value::string(content))]));
                records.extend(
                    outcome::read(name, &arguments, &result)
                        .into_iter()
                        .map(|outcome| Record::Tool(index, outcome)),
                );
            }
            _ => {}
        }
    }
    records
}

fn detailed(records: &[Record]) -> String {
    records
        .iter()
        .map(|record| match record {
            Record::User(index, text) => {
                format!("\n[{index}] User: {}", super::view::excerpt(text, 4096))
            }
            Record::Assistant(index, text) => {
                format!("[{index}] You: {}", super::view::excerpt(text, 2048))
            }
            Record::Tool(index, outcome) => format!("[{index}] {}", outcome.line),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// One turn's lasting facts: what was asked, what was answered and what changed.
#[derive(Default)]
struct Turn<'a> {
    answer: Option<(usize, &'a str)>,
    changed: Vec<&'a str>,
    check: Option<&'a str>,
    failures: usize,
    first_failure: Option<(usize, &'a str)>,
}

impl Turn<'_> {
    fn close(self, level: u8, lines: &mut Vec<String>) {
        let limit = if level == 2 { 480 } else { 140 };
        if let Some((index, text)) = self.answer {
            lines.push(format!(
                "[{index}] You: {}",
                super::view::excerpt(text, limit)
            ));
        }
        if !self.changed.is_empty() {
            lines.push(format!("Changed: {}", self.changed.join(", ")));
        }
        if level == 2
            && let Some(check) = self.check
        {
            lines.push(format!("Last check: {check}"));
        }
        if self.failures > 0 {
            if let Some((index, failure)) = self.first_failure {
                lines.push(format!(
                    "First failed call [{index}]: {}",
                    super::view::excerpt(failure, if level == 2 { 480 } else { 180 })
                ));
            }
            lines.push(format!("Failed calls: {}", self.failures));
        }
    }
}

fn by_turn(records: &[Record], level: u8) -> String {
    let request_limit = if level == 2 { 1024 } else { 320 };
    let mut lines = Vec::new();
    let mut turn = Turn::default();
    for record in records {
        match record {
            Record::User(index, text) => {
                std::mem::take(&mut turn).close(level, &mut lines);
                lines.push(format!(
                    "\n[{index}] User: {}",
                    super::view::excerpt(text, request_limit)
                ));
            }
            Record::Assistant(index, text) => turn.answer = Some((*index, text)),
            Record::Tool(index, outcome) => {
                turn.failures += usize::from(outcome.failed);
                if outcome.failed && turn.first_failure.is_none() {
                    turn.first_failure = Some((*index, &outcome.line));
                }
                if outcome.decision {
                    lines.push(format!(
                        "[{index}] {}",
                        super::view::excerpt(&outcome.line, request_limit)
                    ));
                }
                for path in &outcome.changed {
                    if !turn.changed.contains(&path.as_str()) {
                        turn.changed.push(path);
                    }
                }
                if outcome.command {
                    turn.check = Some(&outcome.line);
                }
            }
        }
    }
    turn.close(level, &mut lines);
    lines.join("\n")
}

#[cfg(test)]
mod tests;
