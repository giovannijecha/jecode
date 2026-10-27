//! A tab's bounded DevTools event log, read in cursor pages.
//!
//! Events are numbered from the first connection to the tab. Eviction keeps
//! the newest `RETAINED` events and is reported as `missed`, so a page never
//! implies a complete browser-session history.
use crate::json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_LOG: AtomicU64 = AtomicU64::new(1);
pub(super) const RETAINED: usize = 1000;
pub(super) const DEFAULT_LIMIT: usize = 50;
pub(super) const MAX_LIMIT: usize = 200;
const MAX_RECORDS: usize = 200;
const RAW_BUDGET_BYTES: usize = 256 * 1024;
const TEXT_LIMIT: usize = 300;
const GAP: &str = "Jecode.eventGap";

pub(super) struct EventLog {
    id: String,
    next: u64,
    capacity: usize,
    events: VecDeque<Value>,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::with_capacity(RETAINED)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Filter {
    Default,
    Console,
    Errors,
    Network,
    All,
}

impl Filter {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        Ok(match value {
            None | Some("default") => Self::Default,
            Some("console") => Self::Console,
            Some("errors") => Self::Errors,
            Some("network") => Self::Network,
            Some("all") => Self::All,
            Some(_) => {
                return Err(
                    "browser logs filter must be default, console, errors, network or all".into(),
                );
            }
        })
    }

    fn matches(self, kind: &str, level: &str) -> bool {
        kind == "gap"
            || match self {
                Self::Default => matches!(kind, "console" | "exception") || level == "error",
                Self::Console => kind == "console",
                Self::Errors => level == "error",
                Self::Network => kind == "network",
                Self::All => true,
            }
    }
}

pub(super) struct Page {
    /// Compact entries that match the filter, at most the requested limit.
    pub entries: Vec<Value>,
    /// Every examined event, matching or not, with its raw CDP message.
    pub records: Vec<Value>,
    pub next_cursor: String,
    /// Retained events after `next_cursor` have not been examined yet.
    pub more: bool,
    /// Events evicted before they could be examined by this read.
    pub missed: u64,
}

impl EventLog {
    pub fn with_capacity(capacity: usize) -> Self {
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            id: format!(
                "{created:x}-{}-{}",
                std::process::id(),
                NEXT_LOG.fetch_add(1, Ordering::Relaxed)
            ),
            next: 0,
            capacity: capacity.max(1),
            events: VecDeque::new(),
        }
    }

    pub fn push(&mut self, event: Value) {
        self.events.push_back(event);
        self.next += 1;
        if self.events.len() > self.capacity {
            self.events.pop_front();
        }
    }

    /// Marks a point where events may have been lost, such as a reconnection.
    pub fn gap(&mut self, reason: &str) {
        self.push(Value::object([
            ("method", Value::string(GAP)),
            ("params", Value::object([("reason", Value::string(reason))])),
        ]));
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    fn first(&self) -> u64 {
        self.next - self.events.len() as u64
    }

    fn cursor(&self, sequence: u64) -> String {
        format!("{}.{sequence}", self.id)
    }

    pub fn page(&self, filter: Filter, limit: usize, cursor: Option<&str>) -> Result<Page, String> {
        let start = match cursor {
            None => 0,
            Some(cursor) => {
                let (log, sequence) = cursor
                    .split_once('.')
                    .and_then(|(log, sequence)| Some((log, sequence.parse::<u64>().ok()?)))
                    .ok_or("Invalid browser logs cursor")?;
                if log != self.id.as_str() {
                    return Err("Browser logs cursor belongs to another tab or browser session; omit cursor to read this tab's retained events".into());
                }
                if sequence > self.next {
                    return Err("Invalid browser logs cursor".into());
                }
                sequence
            }
        };
        let first = self.first();
        let mut page = Page {
            entries: Vec::new(),
            records: Vec::new(),
            next_cursor: self.cursor(self.next),
            more: false,
            missed: first.saturating_sub(start),
        };
        let skip = start.saturating_sub(first) as usize;
        let mut raw_bytes = 0usize;
        for (offset, event) in self.events.iter().enumerate().skip(skip) {
            let sequence = first + offset as u64;
            let event_bytes = event.encode().len();
            if !page.records.is_empty()
                && (page.records.len() == MAX_RECORDS
                    || raw_bytes.saturating_add(event_bytes) > RAW_BUDGET_BYTES)
            {
                page.next_cursor = self.cursor(sequence);
                page.more = true;
                break;
            }
            raw_bytes = raw_bytes.saturating_add(event_bytes);
            let entry = self.entry(sequence, event);
            let mut record = entry.clone();
            if let Value::Object(fields) = &mut record {
                fields.insert("event".into(), event.clone());
            }
            page.records.push(record);
            let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("");
            let level = entry.get("level").and_then(Value::as_str).unwrap_or("");
            if filter.matches(kind, level) {
                page.entries.push(entry);
                if page.entries.len() == limit {
                    page.next_cursor = self.cursor(sequence + 1);
                    page.more = sequence + 1 < self.next;
                    break;
                }
            }
        }
        Ok(page)
    }

    /// A compact, model-readable form of one CDP event.
    fn entry(&self, sequence: u64, event: &Value) -> Value {
        let method = text(event, &["method"]);
        let params = event.get("params").unwrap_or(&Value::Null);
        let (kind, level, message, source) = match method {
            "Runtime.consoleAPICalled" => {
                let level = match text(params, &["type"]) {
                    "error" | "assert" => "error",
                    "warning" => "warning",
                    "info" => "info",
                    "debug" => "debug",
                    _ => "log",
                };
                let message = params
                    .get("args")
                    .and_then(Value::as_array)
                    .unwrap_or(&[])
                    .iter()
                    .map(remote)
                    .collect::<Vec<_>>()
                    .join(" ");
                let source = params
                    .get("stackTrace")
                    .and_then(|trace| trace.get("callFrames"))
                    .and_then(Value::as_array)
                    .and_then(|frames| frames.first())
                    .filter(|frame| !text(frame, &["url"]).is_empty())
                    .map(|frame| location(frame, "url"));
                ("console", level, message, source)
            }
            "Runtime.exceptionThrown" => {
                let details = params.get("exceptionDetails").unwrap_or(&Value::Null);
                let description = text(details, &["exception", "description"]);
                let message = description
                    .lines()
                    .next()
                    .filter(|line| !line.is_empty())
                    .unwrap_or(text(details, &["text"]))
                    .to_string();
                let source =
                    (!text(details, &["url"]).is_empty()).then(|| location(details, "url"));
                ("exception", "error", message, source)
            }
            "Network.loadingFailed" => {
                let url = self.request_url(text(params, &["requestId"]), sequence);
                let canceled = params.get("canceled") == Some(&Value::Bool(true));
                let message = format!("{} {url}", text(params, &["errorText"]));
                let level = if canceled { "info" } else { "error" };
                ("network", level, message.trim().to_string(), None)
            }
            "Network.responseReceived" => {
                let status = params
                    .get("response")
                    .and_then(|response| response.get("status"))
                    .and_then(Value::as_usize)
                    .unwrap_or(0);
                let level = if status >= 400 { "error" } else { "info" };
                let message = format!("{status} {}", text(params, &["response", "url"]));
                ("network", level, message, None)
            }
            "Network.requestWillBeSent" => {
                let message = format!(
                    "{} {}",
                    text(params, &["request", "method"]),
                    text(params, &["request", "url"])
                );
                ("network", "info", message, None)
            }
            "Page.frameNavigated" => {
                let message = format!("navigated to {}", text(params, &["frame", "url"]));
                ("page", "info", message, None)
            }
            "Inspector.targetCrashed" => ("page", "error", "page crashed".into(), None),
            "Inspector.detached" => {
                let message = format!("DevTools detached: {}", text(params, &["reason"]));
                ("page", "warning", message, None)
            }
            GAP => (
                "gap",
                "warning",
                text(params, &["reason"]).to_string(),
                None,
            ),
            _ if method.starts_with("Network.") => ("network", "info", method.into(), None),
            _ if method.starts_with("Page.") => ("page", "info", method.into(), None),
            _ => ("other", "info", method.into(), None),
        };
        let mut entry = Value::object([
            ("seq", Value::number(sequence)),
            ("kind", Value::string(kind)),
            ("level", Value::string(level)),
            ("text", Value::string(bounded(&message))),
        ]);
        if let (Some(source), Value::Object(fields)) = (source, &mut entry) {
            fields.insert("source".into(), Value::string(bounded(&source)));
        }
        entry
    }

    fn request_url(&self, request: &str, sequence: u64) -> &str {
        self.events
            .iter()
            .take((sequence - self.first()) as usize)
            .rfind(|event| {
                text(event, &["method"]) == "Network.requestWillBeSent"
                    && text(event, &["params", "requestId"]) == request
            })
            .map(|event| text(event, &["params", "request", "url"]))
            .unwrap_or("")
    }
}

fn text<'a>(value: &'a Value, path: &[&str]) -> &'a str {
    path.iter()
        .try_fold(value, |value, key| value.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// A console argument as DevTools would print it.
fn remote(value: &Value) -> String {
    match value.get("value") {
        Some(Value::String(text)) => text.clone(),
        Some(value) => value.encode(),
        None => [&["unserializableValue"][..], &["description"], &["type"]]
            .iter()
            .map(|path| text(value, path))
            .find(|text| !text.is_empty())
            .unwrap_or("")
            .to_string(),
    }
}

fn location(frame: &Value, key: &str) -> String {
    let number = |name| frame.get(name).and_then(Value::as_usize).unwrap_or(0) + 1;
    format!(
        "{}:{}:{}",
        text(frame, &[key]),
        number("lineNumber"),
        number("columnNumber")
    )
}

fn bounded(text: &str) -> String {
    if text.chars().count() <= TEXT_LIMIT {
        return text.to_string();
    }
    let mut short: String = text.chars().take(TEXT_LIMIT).collect();
    short.push('…');
    short
}
