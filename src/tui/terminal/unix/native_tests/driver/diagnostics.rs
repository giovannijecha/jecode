//! Failure context for the native Unix terminal fixture.

use super::{Pty, plain};
use crate::json::Value;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, Instant},
};

impl Pty {
    pub(in crate::tui::terminal::unix::native_tests) fn wait_for_terminal_exit(
        &mut self,
        start: usize,
    ) {
        let at = Instant::now();
        loop {
            let visible = plain(&self.bytes);
            let exit = terminal_exit(&visible[start..]);
            if exit == Some(TerminalExit::Zero) {
                return;
            }
            if exit == Some(TerminalExit::Nonzero) || at.elapsed() >= Duration::from_secs(10) {
                let tail = &visible[visible.len().saturating_sub(2048)..];
                panic!(
                    "PTY did not show exited 0; bridge stderr: {:?}; terminal output tail: {}",
                    self.terminal_stderr("native-pty"),
                    String::from_utf8_lossy(tail)
                );
            }
            self.poll();
        }
    }

    fn terminal_stderr(&self, tool_id: &str) -> String {
        let result = (|| {
            // direct_input configures this same home and project directory.
            let store = crate::sessions::Store::new(
                self.directory.path().join("home"),
                self.directory.path(),
            )?;
            let session = store
                .list()?
                .sessions
                .into_iter()
                .next()
                .ok_or("native fixture session is missing")?;
            let saved = store.fixture_load(&session.id)?;
            let reference = stderr_reference(&saved.messages, tool_id)
                .ok_or("native terminal stderr reference is missing")?;
            let outputs = store.outputs(&session.id, crate::redact::Redactor::empty())?;
            let path = outputs.resolve(&reference)?;
            stderr_tail(&path, &reference)
        })();
        result.unwrap_or_else(|error: String| format!("<unavailable: {error}>"))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TerminalExit {
    Zero,
    Nonzero,
}

fn terminal_exit(bytes: &[u8]) -> Option<TerminalExit> {
    bytes.windows(7).enumerate().find_map(|(index, window)| {
        if window != b"exited " {
            return None;
        }
        match bytes.get(index + 7).copied() {
            Some(b'0') => Some(TerminalExit::Zero),
            Some(b'1'..=b'9') => Some(TerminalExit::Nonzero),
            _ => None,
        }
    })
}

fn stderr_reference(messages: &[Value], tool_id: &str) -> Option<String> {
    messages.iter().find_map(|message| {
        if message.get("tool_call_id").and_then(Value::as_str) != Some(tool_id) {
            return None;
        }
        let content = message.get("content").and_then(Value::as_str)?;
        crate::json::parse(content)
            .ok()?
            .get("stderr_file")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

fn stderr_tail(path: &Path, reference: &str) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    let omitted = length.saturating_sub(4096);
    file.seek(SeekFrom::Start(omitted))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity((length - omitted) as usize);
    file.take(4096)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(format!(
        "{reference} ({omitted} earlier bytes omitted): {}",
        String::from_utf8_lossy(&bytes)
    ))
}

#[test]
fn terminal_diagnostic_reads_only_matching_stderr_tail() {
    let directory = crate::test_support::Directory::new();
    let path = directory.path().join("bridge.stderr");
    let mut content = vec![b'x'; 8192];
    content.extend_from_slice(b"Could not confirm Unix PTY session cleanup\n");
    std::fs::write(&path, content).unwrap();
    let messages = [
        Value::object([
            ("tool_call_id", Value::string("other")),
            (
                "content",
                Value::string(r#"{"stderr_file":"output:other:stderr"}"#),
            ),
        ]),
        Value::object([
            ("tool_call_id", Value::string("native-pty")),
            (
                "content",
                Value::string(r#"{"stderr_file":"output:bridge:stderr"}"#),
            ),
        ]),
    ];
    let reference = stderr_reference(&messages, "native-pty").unwrap();
    let diagnostic = stderr_tail(&path, &reference).unwrap();
    assert!(diagnostic.starts_with("output:bridge:stderr ("));
    assert!(diagnostic.contains(" earlier bytes omitted): "));
    assert!(diagnostic.ends_with("Could not confirm Unix PTY session cleanup\n"));
    assert!(diagnostic.len() < 4200);
}

#[test]
fn terminal_exit_waits_for_first_status_digit() {
    assert_eq!(terminal_exit(b""), None);
    assert_eq!(terminal_exit(b"command exited "), None);
    assert_eq!(terminal_exit(b"command exited 0"), Some(TerminalExit::Zero));
    assert_eq!(
        terminal_exit(b"command exited 125"),
        Some(TerminalExit::Nonzero)
    );
    let mut split = b"command exited ".to_vec();
    assert_eq!(terminal_exit(&split), None);
    split.extend_from_slice(b"0");
    assert_eq!(terminal_exit(&split), Some(TerminalExit::Zero));
}
