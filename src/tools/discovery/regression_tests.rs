use super::*;
use crate::test_support::Directory;
use std::fs;

#[test]
fn a_large_absent_query_cannot_expand_the_result_beyond_the_tool_budget() {
    let project = Directory::new();
    let root = fs::canonicalize(project.path()).unwrap();
    fs::write(project.path().join("source.txt"), "ordinary text\n").unwrap();
    let result = execute(
        &root,
        "search",
        &Value::object([("query", Value::string("x".repeat(70 * 1024)))]),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(result.get("matches_returned"), Some(&Value::number(0)));
    assert_eq!(result.get("complete"), Some(&Value::Bool(true)));
    assert!(result.encode().len() <= super::super::OUTPUT_LIMIT);
}

#[test]
fn complete_preview_at_the_boundary_excludes_line_breaks_from_truncation() {
    for newline in ["\n", "\r\n"] {
        let project = Directory::new();
        let root = fs::canonicalize(project.path()).unwrap();
        fs::write(
            project.path().join("source.txt"),
            format!("{}{}", "x".repeat(1024), newline),
        )
        .unwrap();
        let result = execute(
            &root,
            "search",
            &Value::object([("query", Value::string("x"))]),
            &Cancellation::default(),
        )
        .unwrap();
        let row = &result.get("matches").unwrap().as_array().unwrap()[0];
        assert_eq!(row.get("text").and_then(Value::as_str).unwrap().len(), 1024);
        assert_eq!(row.get("text_truncated"), Some(&Value::Bool(false)));
    }
}

#[test]
fn a_match_too_large_for_one_page_is_returned_clipped_not_refused() {
    // Escaped control characters make each context line encode about six
    // times its length, so one match with context exceeds a page.
    let project = Directory::new();
    let root = fs::canonicalize(project.path()).unwrap();
    let noisy = "\u{1}".repeat(1000);
    let mut text = String::new();
    for _ in 0..10 {
        text.push_str(&noisy);
        text.push('\n');
    }
    text.push_str(&noisy);
    text.push_str("needle\n");
    for _ in 0..10 {
        text.push_str(&noisy);
        text.push('\n');
    }
    fs::write(project.path().join("minified.txt"), text).unwrap();
    let result = execute(
        &root,
        "search",
        &Value::object([
            ("query", Value::string("needle")),
            ("context", Value::number(10)),
        ]),
        &Cancellation::default(),
    )
    .unwrap();
    let matches = result.get("matches").and_then(Value::as_array).unwrap();
    assert_eq!(matches.len(), 1, "{}", result.encode().len());
    let found = &matches[0];
    assert_eq!(found.get("path"), Some(&Value::string("minified.txt")));
    assert_eq!(found.get("line"), Some(&Value::number(11)));
    assert_eq!(found.get("clipped"), Some(&Value::Bool(true)));
    assert_eq!(found.get("text_truncated"), Some(&Value::Bool(true)));
    let before = found.get("before").and_then(Value::as_array).unwrap();
    assert_eq!(before[0].get("text_truncated"), Some(&Value::Bool(true)));
    let after = found.get("after").and_then(Value::as_array).unwrap();
    assert_eq!(after[0].get("text_truncated"), Some(&Value::Bool(true)));
    let read = crate::tools::files::read_page(
        &root.join("minified.txt"),
        &Value::object([("byte_offset", found.get("byte_offset").unwrap().clone())]),
        &Cancellation::default(),
    )
    .unwrap();
    assert!(
        read.get("content")
            .and_then(Value::as_str)
            .unwrap()
            .starts_with("needle")
    );
    assert!(result.encode().len() <= super::super::OUTPUT_LIMIT);
}

#[test]
fn page_limits_past_their_maximum_return_a_full_page_instead_of_an_error() {
    let project = Directory::new();
    let root = fs::canonicalize(project.path()).unwrap();
    let lines: String = (1..=2500).map(|line| format!("line {line}\n")).collect();
    fs::write(project.path().join("long.txt"), lines).unwrap();
    let listed = execute(
        &root,
        "list",
        &Value::object([("limit", Value::number(5000))]),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(listed.get("entries_returned"), Some(&Value::number(1)));
    let tools = crate::tools::Tools::new(&root).unwrap();
    let read = tools.execute(
        "read",
        &Value::object([
            ("path", Value::string("long.txt")),
            ("limit", Value::number(5000)),
        ])
        .encode(),
    );
    assert!(read.get("error").is_none(), "{read:?}");
    assert_eq!(read.get("next_offset"), Some(&Value::number(2001)));
}
