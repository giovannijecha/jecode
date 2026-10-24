use super::*;
use crate::redact::Redactor;
use crate::test_support::Directory;
use crate::tools::Tools;

fn call(tools: &Tools, name: &str, arguments: Value) -> Value {
    tools.execute(name, &arguments.encode())
}

fn number(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_usize).unwrap()
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap()
}

fn poll(tools: &Tools, id: &str, limit: usize) -> Value {
    call(
        tools,
        "process",
        Value::object([
            ("action", Value::string("poll")),
            ("process_id", Value::string(id)),
            ("wait_ms", Value::number(10_000)),
            ("output_limit_bytes", Value::number(limit)),
        ]),
    )
}

#[test]
fn process_pages_follow_cursors_and_allow_a_budget_above_sixty_four_kib() {
    let directory = Directory::new();
    let mut tools = Tools::new(directory.path()).unwrap();
    tools.configure_output(directory.path().join("logs"), Redactor::empty());
    let started = call(
        &tools,
        "bash",
        Value::object([
            ("command", Value::string("printf '%90000s' x")),
            ("wait_ms", Value::number(0)),
            ("output_limit_bytes", Value::number(1)),
        ]),
    );
    let id = text(&started, "process_id");
    let mut output = text(&started, "stdout").to_string();
    let mut next = number(&started, "stdout_next_offset");
    assert_eq!(number(&started, "output_limit_bytes"), 1);

    let invalid = poll(&tools, id, 0);
    assert!(text(&invalid, "error").contains("output_limit_bytes"));
    let large = poll(&tools, id, 70_000);
    assert_eq!(number(&large, "stdout_offset"), next);
    assert_eq!(number(&large, "stdout_skipped_bytes"), 0);
    assert_eq!(number(&large, "output_limit_bytes"), 70_000);
    output.push_str(text(&large, "stdout"));
    next = number(&large, "stdout_next_offset");
    assert_eq!(text(&large, "stdout").len(), 70_000);
    assert_eq!(number(&large, "stdout_remaining_bytes"), 90_000 - next);

    for _ in 0..10 {
        if next == 90_000 {
            break;
        }
        let page = poll(&tools, id, DEFAULT_OUTPUT_LIMIT);
        assert_eq!(number(&page, "stdout_offset"), next);
        assert_eq!(number(&page, "stdout_skipped_bytes"), 0);
        output.push_str(text(&page, "stdout"));
        next = number(&page, "stdout_next_offset");
    }
    assert_eq!(next, 90_000);
    assert_eq!(output.len(), 90_000);
    assert!(output.ends_with('x'));
    let drained = poll(&tools, id, DEFAULT_OUTPUT_LIMIT);
    assert_eq!(text(&drained, "stdout"), "");
    assert_eq!(number(&drained, "stdout_next_offset"), 90_000);
    assert_eq!(number(&drained, "stdout_remaining_bytes"), 0);
}

#[test]
fn oversized_poll_budget_is_bounded_without_skipping_the_saved_backlog() {
    let directory = Directory::new();
    let mut tools = Tools::new(directory.path()).unwrap();
    tools.configure_output(directory.path().join("logs"), Redactor::empty());
    let started = call(
        &tools,
        "bash",
        Value::object([
            ("command", Value::string("printf '%5000000s' x")),
            ("wait_ms", Value::number(0)),
            ("output_limit_bytes", Value::number(1)),
        ]),
    );
    let from = number(&started, "stdout_next_offset");
    let page = poll(&tools, text(&started, "process_id"), 100_000_000);
    assert_eq!(number(&page, "output_limit_bytes"), 256 * 1024);
    assert_eq!(text(&page, "stdout").len(), 256 * 1024);
    assert_eq!(number(&page, "stdout_offset"), from);
    assert_eq!(number(&page, "stdout_skipped_bytes"), 0);
    assert_eq!(
        number(&page, "stdout_remaining_bytes"),
        5_000_000 - from - 256 * 1024
    );
    let saved = tools.outputs.resolve(text(&page, "stdout_file")).unwrap();
    assert_eq!(std::fs::metadata(saved).unwrap().len(), 5_000_000);
}

#[test]
fn explicit_tail_poll_reaches_readiness_and_reports_the_skipped_backlog() {
    let directory = Directory::new();
    let mut tools = Tools::new(directory.path()).unwrap();
    tools.configure_output(directory.path().join("logs"), Redactor::empty());
    let started = call(
        &tools,
        "bash",
        Value::object([
            (
                "command",
                Value::string("printf '%90000s' x; printf '\nready\n'"),
            ),
            ("wait_ms", Value::number(0)),
            ("output_limit_bytes", Value::number(1)),
        ]),
    );
    let from = number(&started, "stdout_next_offset");
    let id = text(&started, "process_id");
    let tail = call(
        &tools,
        "process",
        Value::object([
            ("action", Value::string("poll")),
            ("process_id", Value::string(id)),
            ("wait_ms", Value::number(10_000)),
            ("output_limit_bytes", Value::number(256)),
            ("output_mode", Value::string("tail")),
        ]),
    );
    assert!(text(&tail, "stdout").ends_with("\nready\n"));
    assert_eq!(text(&tail, "output_mode"), "tail");
    assert!(number(&tail, "stdout_offset") > from);
    assert_eq!(
        number(&tail, "stdout_skipped_bytes"),
        number(&tail, "stdout_offset") - from
    );
    assert_eq!(number(&tail, "stdout_remaining_bytes"), 0);
    let next = poll(&tools, id, DEFAULT_OUTPUT_LIMIT);
    assert_eq!(text(&next, "stdout"), "");
    assert_eq!(
        number(&next, "stdout_offset"),
        number(&tail, "stdout_next_offset")
    );
    let saved = tools.outputs.resolve(text(&tail, "stdout_file")).unwrap();
    assert_eq!(std::fs::metadata(saved).unwrap().len(), 90_007);
}

#[test]
fn stop_uses_its_own_budget_without_advancing_past_unread_output() {
    let directory = Directory::new();
    let mut tools = Tools::new(directory.path()).unwrap();
    tools.configure_output(directory.path().join("logs"), Redactor::empty());
    let started = call(
        &tools,
        "bash",
        Value::object([
            ("command", Value::string("printf '%20000s' x")),
            ("wait_ms", Value::number(0)),
            ("output_limit_bytes", Value::number(1)),
        ]),
    );
    let id = text(&started, "process_id");
    let initial_next = number(&started, "stdout_next_offset");
    let finished = poll(&tools, id, 1);
    assert_eq!(text(&finished, "status"), "exited");
    let before_stop = number(&finished, "stdout_next_offset");
    assert!(before_stop >= initial_next);
    let stopped = call(
        &tools,
        "process",
        Value::object([
            ("action", Value::string("stop")),
            ("process_id", Value::string(id)),
            ("output_limit_bytes", Value::number(1_024)),
        ]),
    );
    assert_eq!(number(&stopped, "output_limit_bytes"), 1_024);
    assert_eq!(number(&stopped, "stdout_offset"), before_stop);
    assert_eq!(text(&stopped, "stdout").len(), 1_024);
    assert_eq!(number(&stopped, "stdout_skipped_bytes"), 0);
    let next = poll(&tools, id, DEFAULT_OUTPUT_LIMIT);
    assert_eq!(
        number(&next, "stdout_offset"),
        number(&stopped, "stdout_next_offset")
    );
}

#[test]
fn bounded_reads_respect_utf8_boundaries_without_skipping_bytes() {
    let directory = Directory::new();
    let path = directory.path().join("stream");
    // One partial character at the end of a live stream waits for the next read.
    std::fs::write(&path, &"aé".as_bytes()[..2]).unwrap();
    let live = read_after(&path, 0, false, DEFAULT_OUTPUT_LIMIT).unwrap();
    assert_eq!((live.text.as_str(), live.next), ("a", 1));
    let finished = read_after(&path, 1, true, DEFAULT_OUTPUT_LIMIT).unwrap();
    assert_eq!(finished.text, "\u{fffd}");
    // Consecutive windows continue from the same byte without splitting a character.
    let bytes = "é".repeat(DEFAULT_OUTPUT_LIMIT).into_bytes();
    std::fs::write(&path, &bytes).unwrap();
    let mut next = 0;
    let mut joined = String::new();
    while next < bytes.len() as u64 {
        let chunk = read_after(&path, next, false, DEFAULT_OUTPUT_LIMIT - 1).unwrap();
        assert_eq!(chunk.offset, next);
        assert!(chunk.next > next);
        next = chunk.next;
        joined += &chunk.text;
    }
    assert_eq!(joined, "é".repeat(DEFAULT_OUTPUT_LIMIT));
    let tiny = read_after(&path, 0, false, 1).unwrap();
    assert_eq!((tiny.text.as_str(), tiny.next), ("é", 2));
}
