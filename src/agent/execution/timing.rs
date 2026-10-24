use super::{Agent, ToolCall, Value};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub(super) struct Clock {
    started: Instant,
    unix_ms: u128,
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
