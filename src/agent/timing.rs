use super::{Agent, ToolCall, Value};
use crate::{backend::Usage, effort::Effort};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub(super) struct Clock {
    pub(super) started: Instant,
    pub(super) unix_ms: u128,
}

/// One model attempt, including local request projection before backend work.
pub(super) struct RequestTiming {
    pub(super) clock: Clock,
    pub(super) dispatched: Option<Instant>,
    pub(super) first_update: Option<Instant>,
    pub(super) request_bytes: Option<usize>,
    pub(super) notification: Duration,
}

impl RequestTiming {
    pub(super) fn start() -> Self {
        Self {
            clock: Clock::new(),
            dispatched: None,
            first_update: None,
            request_bytes: None,
            notification: Duration::ZERO,
        }
    }

    /// A single provider attempt. Counters are snapshots, never accumulated
    /// across streaming updates or retries.
    pub(super) fn event(
        self,
        operation: &str,
        (backend, model, effort): (&str, &str, Effort),
        attempt: usize,
        status: &str,
        usage: Option<Usage>,
        after_message: usize,
    ) -> Value {
        let finished = Instant::now();
        let milliseconds = |duration: Duration| Value::number(duration.as_millis());
        let since = |later: Instant, earlier: Instant| {
            milliseconds(later.saturating_duration_since(earlier))
        };
        let tokens = |field: fn(Usage) -> Option<usize>| {
            usage.and_then(field).map_or(Value::Null, Value::number)
        };
        Value::object([
            ("type", Value::string("request_timing")),
            ("operation", Value::string(operation)),
            ("backend", Value::string(backend)),
            ("model", Value::string(model)),
            ("effort", Value::string(effort.name())),
            ("attempt", Value::number(attempt)),
            ("started_at_unix_ms", Value::number(self.clock.unix_ms)),
            (
                "request_bytes",
                self.request_bytes.map_or(Value::Null, Value::number),
            ),
            ("status", Value::string(status)),
            ("input_tokens", tokens(|usage| usage.input_tokens)),
            ("output_tokens", tokens(|usage| usage.output_tokens)),
            ("total_tokens", tokens(|usage| usage.total_tokens)),
            ("cached_tokens", tokens(|usage| usage.cached_tokens)),
            (
                "cache_write_tokens",
                tokens(|usage| usage.cache_write_tokens),
            ),
            ("reasoning_tokens", tokens(|usage| usage.reasoning_tokens)),
            (
                "preparation_ms",
                self.dispatched.map_or(Value::Null, |dispatched| {
                    milliseconds(
                        dispatched
                            .saturating_duration_since(self.clock.started)
                            .saturating_sub(self.notification),
                    )
                }),
            ),
            ("notification_ms", milliseconds(self.notification)),
            (
                "first_update_ms",
                self.dispatched
                    .zip(self.first_update)
                    .map_or(Value::Null, |(dispatched, first)| since(first, dispatched)),
            ),
            (
                "response_ms",
                self.dispatched
                    .map_or(Value::Null, |dispatched| since(finished, dispatched)),
            ),
            ("total_ms", since(finished, self.clock.started)),
            ("after_message", Value::number(after_message)),
        ])
    }
}

impl Clock {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        }
    }

    pub(super) fn finish(self, executing: Instant) -> Value {
        let finished = Instant::now();
        Value::object([
            ("type", Value::string("tool_timing")),
            ("started_at_unix_ms", Value::number(self.unix_ms)),
            (
                "preparation_ms",
                Value::number(executing.duration_since(self.started).as_millis()),
            ),
            (
                "execution_ms",
                Value::number(finished.duration_since(executing).as_millis()),
            ),
            (
                "total_ms",
                Value::number(finished.duration_since(self.started).as_millis()),
            ),
        ])
    }
}

impl Agent {
    pub(super) fn record_tool_timing(&self, call: &ToolCall, mut timing: Value) {
        if let Value::Object(fields) = &mut timing {
            fields.insert("tool_call_id".into(), Value::string(self.redact(&call.id)));
            fields.insert("name".into(), Value::string(self.redact(&call.name)));
            fields.insert(
                "after_message".into(),
                Value::number(self.messages.lock().unwrap().len()),
            );
        }
        self.events.lock().unwrap().push(timing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::openrouter::OpenRouter,
        test_support::{Directory, HttpFixture, completion, tool_call},
        tools::Tools,
    };

    #[test]
    fn parallel_timings_follow_results_even_when_call_ids_repeat() {
        let project = Directory::new();
        std::fs::write(project.path().join("a.txt"), "first").unwrap();
        std::fs::write(project.path().join("b.txt"), "second").unwrap();
        let mut agent = Agent::new(
            OpenRouter::fixture("http://127.0.0.1:1".into()),
            Tools::new(project.path()).unwrap(),
        );
        let calls: Vec<_> = ["a.txt", "b.txt"]
            .into_iter()
            .map(|path| ToolCall {
                id: "repeated".into(),
                name: "read".into(),
                arguments: Value::object([("path", Value::string(path))]).encode(),
            })
            .collect();
        agent.execute_calls(&calls, &mut |_| Ok(())).unwrap();
        let document = agent.archive().document();
        let timings: Vec<_> = document
            .get("events")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter(|event| event.get("type").and_then(Value::as_str) == Some("tool_timing"))
            .collect();
        assert_eq!(timings.len(), 2);
        assert_ne!(
            timings[0].get("after_message"),
            timings[1].get("after_message")
        );
        let messages = document.get("messages").and_then(Value::as_array).unwrap();
        for (timing, content) in timings.iter().zip(["first", "second"]) {
            let end = timing
                .get("after_message")
                .and_then(Value::as_usize)
                .unwrap();
            assert!(
                messages[end - 1]
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap()
                    .contains(content)
            );
        }
    }

    #[test]
    fn serial_and_parallel_timings_survive_storage_without_entering_model_history() {
        let project = Directory::new();
        let home = Directory::new();
        std::fs::write(project.path().join("a.txt"), "old").unwrap();
        let calls = vec![
            tool_call(
                "read",
                "read",
                Value::object([("path", Value::string("a.txt"))]),
            ),
            tool_call(
                "missing",
                "read",
                Value::object([("path", Value::string("missing.txt"))]),
            ),
            tool_call(
                "write",
                "write",
                Value::object([
                    ("path", Value::string("a.txt")),
                    ("content", Value::string("new")),
                ]),
            ),
        ];
        let server = HttpFixture::new(vec![
            (200, completion("", calls)),
            (200, completion("Done.", vec![])),
        ]);
        let mut agent = Agent::new(
            OpenRouter::fixture(server.endpoint.clone()),
            Tools::new(project.path()).unwrap(),
        );
        agent.enable_sessions(home.path()).unwrap();
        agent
            .run_turn("Read both, then write.", &mut |_| Ok(()))
            .unwrap();
        let saved = agent.sessions().unwrap().snapshot();
        let timings: Vec<_> = saved
            .events
            .iter()
            .filter(|event| event.get("type").and_then(Value::as_str) == Some("tool_timing"))
            .collect();
        assert_eq!(timings.len(), 3);
        for (timing, id) in timings.iter().zip(["read", "missing", "write"]) {
            assert_eq!(timing.get("tool_call_id").and_then(Value::as_str), Some(id));
            assert!(
                timing
                    .get("started_at_unix_ms")
                    .and_then(Value::as_usize)
                    .unwrap()
                    > 0
            );
            let count = timing
                .get("after_message")
                .and_then(Value::as_usize)
                .unwrap();
            assert_eq!(
                saved.messages[count - 1]
                    .get("tool_call_id")
                    .and_then(Value::as_str),
                Some(id)
            );
            let total = timing.get("total_ms").and_then(Value::as_usize).unwrap();
            for field in ["preparation_ms", "execution_ms"] {
                assert!(timing.get(field).and_then(Value::as_usize).unwrap() <= total);
            }
        }
        let exported = agent.archive().document();
        let exported_events = exported.get("events").and_then(Value::as_array).unwrap();
        assert!(
            timings
                .iter()
                .all(|timing| exported_events.contains(timing))
        );
        for request in server.finish() {
            assert!(
                !request
                    .body
                    .get("messages")
                    .unwrap()
                    .encode()
                    .contains("tool_timing")
            );
        }
    }
}
