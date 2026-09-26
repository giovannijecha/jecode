//! Explicit, bounded export of safe attempt metadata from a closed session.
use jecode::session::{
    persistence::{self, DiagnosticAttempt},
    scope::Directory,
};
use std::{
    io::{self, Write},
    path::Path,
};

fn operation(value: Option<&str>) -> &str {
    match value {
        Some("host resolution") => "resolve",
        Some("TCP connect") => "connect",
        Some("read timeout setup") => "configure_read",
        Some("write timeout setup") => "configure_write",
        Some("TCP option setup") => "configure_tcp",
        Some("TLS record write") => "record_write",
        Some("TLS record header read") => "record_header_read",
        Some("TLS record body read") => "record_body_read",
        _ => "none_or_unknown",
    }
}
fn category(value: Option<&str>) -> &str {
    match value {
        Some("ConnectionReset") => "ConnectionReset",
        Some("ConnectionRefused") => "ConnectionRefused",
        Some("ConnectionAborted") => "ConnectionAborted",
        Some("BrokenPipe") => "BrokenPipe",
        Some("TimedOut") => "TimedOut",
        Some("WouldBlock") => "WouldBlock",
        Some("Interrupted") => "Interrupted",
        Some("UnexpectedEof") => "UnexpectedEof",
        _ => "none_or_unknown",
    }
}
fn line(record: &DiagnosticAttempt) -> String {
    let attempt = &record.attempt;
    format!(
        "turn={} source={} request_in_command={} connection_attempt={} delivery={} stage={} stage_ms={} request_ms={} since_progress_ms={} termination={} accepted_tls_write_bytes={} received_tls_wire_bytes={} decrypted_http_bytes={} http_status={} sse_events={} operation={} category={} os_code={} retrying={}",
        record.turn,
        record.source.name(),
        attempt.request_sequence,
        attempt.connection_attempt,
        attempt.delivery.name(),
        attempt.stage.map_or("none", |stage| stage.name()),
        attempt.stage_elapsed_ms,
        attempt.request_elapsed_ms,
        attempt
            .since_progress_ms
            .map_or("none".into(), |elapsed| elapsed.to_string()),
        attempt.termination.map_or("none", |kind| kind.name()),
        attempt.accepted_wire_bytes,
        attempt.received_wire_bytes,
        attempt.response_plaintext_bytes,
        attempt
            .response_status
            .map_or("none".into(), |status| status.to_string()),
        attempt.stream_events,
        operation(attempt.operation.as_deref()),
        category(attempt.category.as_deref()),
        attempt
            .os_code
            .map_or("none".into(), |code| code.to_string()),
        attempt.retrying,
    )
}
pub(super) fn run(id: &str, directory: &Path) -> Result<(), String> {
    let directory = Directory::open(directory).map_err(|_| "cannot open selected directory")?;
    let attempts = persistence::recent_network_attempts_in(id, &directory)
        .map_err(|_| "cannot inspect a closed session in the selected directory")?;
    let mut output = io::stdout().lock();
    for attempt in &attempts {
        writeln!(output, "{}", line(attempt)).map_err(|_| "cannot write diagnostics")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jecode::providers::openai_account::client::{Attempt, Termination};
    #[test]
    fn export_uses_only_fixed_labels_and_bounded_numbers() {
        let attempt = Attempt {
            operation: Some("synthetic secret prompt".into()),
            category: Some("synthetic secret response".into()),
            diagnostic: Some("synthetic secret token".into()),
            os_code: Some(10054),
            request_elapsed_ms: 315_000,
            since_progress_ms: Some(300_000),
            termination: Some(Termination::IdleTimeout),
            ..Default::default()
        };
        let output = line(&DiagnosticAttempt {
            turn: 7,
            source: persistence::AttemptSource::Compaction,
            attempt,
        });
        assert!(output.contains("turn=7 source=compaction request_in_command=0"));
        assert!(output.contains("os_code=10054"));
        assert!(output.contains("operation=none_or_unknown"));
        assert!(output.contains(
            "request_ms=315000 since_progress_ms=300000 termination=stream_idle_timeout"
        ));
        assert!(!output.contains("synthetic secret"));
        assert!(output.len() < 512);
    }
}
