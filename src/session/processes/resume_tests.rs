use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::json::Value;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use crate::tools::Tools;
use std::time::{Duration, Instant};

const FAILURE: &str = "plain-process-auth-failure";

/// Ends the command while the failed turn is being reported, before the loop
/// decides whether another model request is allowed.
struct ReleaseOnFailure {
    bytes: Vec<u8>,
    observer: Observer,
    gate: std::path::PathBuf,
    released: bool,
}

impl std::io::Write for ReleaseOnFailure {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        if !self.released && String::from_utf8_lossy(&self.bytes).contains(FAILURE) {
            self.released = true;
            std::fs::write(&self.gate, "go")?;
            let started = Instant::now();
            while self.observer.completions_since(0).1.is_empty() {
                if started.elapsed() >= Duration::from_secs(10) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "The fixture command did not finish",
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_failed_plain_turn_holds_process_continuation_during_local_inspection() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "start",
                    "bash",
                    Value::object([
                        (
                            "command",
                            Value::string("until [ -e go ]; do sleep 0.02; done; printf finished"),
                        ),
                        ("wait_ms", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (
            401,
            Value::object([(
                "error",
                Value::object([("message", Value::string(FAILURE))]),
            )]),
        ),
        (
            200,
            completion("Unexpected automatic continuation.", vec![]),
        ),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    let mut config = crate::session::SessionConfig {
        store: crate::config::Store::new(home.path().join("config.json")),
        settings: crate::config::Settings::new(
            "isolated-fixture-key".into(),
            "fixture/model".into(),
        )
        .unwrap(),
    };
    let mut status = ReleaseOnFailure {
        bytes: Vec::new(),
        observer: agent.processes(),
        gate: project.path().join("go"),
        released: false,
    };
    let mut output = Vec::new();
    crate::session::chat(
        &mut agent,
        &mut config,
        &mut std::io::Cursor::new("Build it\n\n/process\n/exit\n"),
        &mut output,
        &mut status,
        Vec::new(),
    )
    .unwrap();
    assert!(status.released, "The fixture must observe the failed turn");
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("exited 0"), "{output}");
    assert!(!output.contains("Unexpected automatic continuation."));
    assert_eq!(fixture.finish_received().len(), 2);
}

/// Hands out `lines` one read at a time, holding the second until the
/// process has ended, as a person typing later would.
struct LaterLines {
    lines: Vec<&'static str>,
    current: Vec<u8>,
    position: usize,
    ended: Observer,
}

impl std::io::Read for LaterLines {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use std::io::BufRead;
        let available = self.fill_buf()?;
        let length = available.len().min(buffer.len());
        buffer[..length].copy_from_slice(&available[..length]);
        self.consume(length);
        Ok(length)
    }
}

impl std::io::BufRead for LaterLines {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.position == self.current.len() && !self.lines.is_empty() {
            if self.lines.len() == 2 {
                let started = Instant::now();
                while self.ended.completions_since(0).1.is_empty() {
                    assert!(started.elapsed() < Duration::from_secs(10));
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            self.current = self.lines.remove(0).as_bytes().to_vec();
            self.position = 0;
        }
        Ok(&self.current[self.position..])
    }

    fn consume(&mut self, amount: usize) {
        self.position += amount;
    }
}

#[test]
fn plain_chat_continues_at_its_next_line_and_lists_what_finished() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "bash-1",
                    "bash",
                    Value::object([
                        ("command", Value::string("sleep 0.2; printf plain-done")),
                        ("wait_ms", Value::number(0)),
                    ]),
                )],
            ),
        ),
        (200, completion("Started.", vec![])),
        (200, completion("It finished.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    let mut config = crate::session::SessionConfig {
        store: crate::config::Store::new(home.path().join("config.json")),
        settings: crate::config::Settings::new(
            "isolated-fixture-key".into(),
            "fixture/model".into(),
        )
        .unwrap(),
    };
    let mut input = LaterLines {
        lines: vec!["Build it\n", "\n", "/process\n", "/exit\n"],
        current: Vec::new(),
        position: 0,
        ended: agent.processes(),
    };
    let mut output = Vec::new();
    let mut status = Vec::new();
    crate::session::chat(
        &mut agent,
        &mut config,
        &mut input,
        &mut output,
        &mut status,
        Vec::new(),
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    let status = String::from_utf8(status).unwrap();
    assert!(output.contains("It finished."), "{output}");
    assert!(
        output.contains("Processes (0 running, 1 recent):"),
        "{output}"
    );
    assert!(output.contains("exited 0"), "{output}");
    assert!(status.contains("[process: exit 0] sleep 0.2"), "{status}");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0]
            .body
            .encode()
            .contains(r#"\"background_resume\":\"on_input\""#),
        "{}",
        requests[0].body.encode()
    );
    assert!(
        crate::test_support::latest_state(&requests[2].body)
            .contains("Processes that finished, each shown once:")
    );
}
