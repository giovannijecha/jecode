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
