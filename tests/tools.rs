#[path = "support/workspace.rs"]
mod support;
use jecode::{
    json::{self, Value},
    tools::Prepared,
    workspace::{Budget, Workspace},
};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use support::Fixture;

fn execute(workspace: &Workspace, tool: &str, args: &str) -> Value {
    let args = json::parse(args, Default::default()).unwrap();
    let prepared = Prepared::parse(tool, &args).unwrap();
    let cancelled = AtomicBool::new(false);
    let output = prepared.execute(
        workspace,
        &Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(10),
        },
    );
    assert!(output.text.len() <= jecode::tools::MAX_OUTPUT);
    let value = json::parse(&output.text, Default::default()).unwrap();
    let limited = value.get("truncated") == Some(&Value::Bool(true))
        || value.get("omitted").and_then(Value::unsigned).unwrap_or(0) > 0;
    assert_eq!(
        output.limited, limited,
        "presentation must retain truncation/omission metadata"
    );
    value
}

#[test]
fn read_lines_are_exact_and_paginate_without_losing_content() {
    let fixture = Fixture::new();
    fixture.write("text", "first\r\nsecond café\nlast");
    fixture.write("empty", "");
    fixture.write("long", "a".repeat(8193));
    let workspace = Workspace::open(&fixture.0).unwrap();
    let result = execute(&workspace, "read_file", r#"{"path":"text","max_lines":2}"#);
    assert_eq!(
        result.get("text").and_then(Value::text),
        Some("first\r\nsecond café\n")
    );
    assert_eq!(result.get("next_line").and_then(Value::unsigned), Some(3));
    let result = execute(&workspace, "read_file", r#"{"path":"text","start_line":3}"#);
    assert_eq!(result.get("text").and_then(Value::text), Some("last"));
    assert_eq!(result.get("next_line"), Some(&Value::Null));
    for args in [r#"{"path":"empty"}"#, r#"{"path":"text","start_line":10}"#] {
        let result = execute(&workspace, "read_file", args);
        assert_eq!(result.get("text").and_then(Value::text), Some(""));
        assert_eq!(result.get("end_line"), Some(&Value::Null));
    }
    assert_eq!(
        execute(&workspace, "read_file", r#"{"path":"long"}"#).get("ok"),
        Some(&Value::Bool(false))
    );
}

#[test]
fn read_file_pages_an_existing_file_beyond_one_mib() {
    let fixture = Fixture::new();
    let mut content = "plain\n".repeat(200_000);
    content.push_str("\tselected 世界\r\nnext line");
    assert!(content.len() > 1024 * 1024);
    fixture.write("large.txt", &content);
    let workspace = Workspace::open(&fixture.0).unwrap();
    let result = execute(
        &workspace,
        "read_file",
        r#"{"path":"large.txt","start_line":200001,"max_lines":1}"#,
    );
    assert_eq!(
        result.get("text").and_then(Value::text),
        Some("\tselected 世界\r\n")
    );
    assert_eq!(
        result.get("next_line").and_then(Value::unsigned),
        Some(200_002)
    );
    let result = execute(
        &workspace,
        "read_file",
        r#"{"path":"large.txt","start_line":200002,"max_lines":1}"#,
    );
    assert_eq!(result.get("text").and_then(Value::text), Some("next line"));
    assert_eq!(result.get("next_line"), Some(&Value::Null));
}

#[test]
fn directory_and_search_results_are_truthful_when_limited() {
    let fixture = Fixture::new();
    fixture.write("a.txt", "needle first\nneedle again\n");
    fixture.write("nested/b.txt", "unrelated\nneedle café\n");
    fixture.write("binary", b"needle\0");
    fixture.write(".env", "needle secret");
    let workspace = Workspace::open(&fixture.0).unwrap();
    let result = execute(&workspace, "list_files", r#"{"limit":1}"#);
    assert_eq!(
        result.get("entries").and_then(Value::array).unwrap().len(),
        1
    );
    assert_eq!(result.get("truncated"), Some(&Value::Bool(true)));
    let result = execute(&workspace, "search_text", r#"{"query":"needle"}"#);
    let matches = result.get("matches").and_then(Value::array).unwrap();
    assert_eq!(matches.len(), 3);
    assert_eq!(
        matches[2].get("path").and_then(Value::text),
        Some("nested/b.txt")
    );
    assert_eq!(matches[2].get("line").and_then(Value::unsigned), Some(2));
    assert_eq!(result.get("omitted").and_then(Value::unsigned), Some(2));
    assert_eq!(result.get("truncated"), Some(&Value::Bool(false)));
    let limited = execute(
        &workspace,
        "search_text",
        r#"{"query":"needle","max_results":1}"#,
    );
    assert_eq!(limited.get("path").and_then(Value::text), Some("."));
    assert_eq!(limited.get("truncated"), Some(&Value::Bool(true)));
    assert_eq!(
        limited.get("matches").and_then(Value::array).unwrap().len(),
        1
    );
    let miss = execute(&workspace, "search_text", r#"{"query":"NEEDLE"}"#);
    assert!(
        miss.get("matches")
            .and_then(Value::array)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn long_line_excerpt_keeps_the_match_and_byte_column() {
    let fixture = Fixture::new();
    fixture.write(
        "unicode",
        format!("{}needle{}", "界".repeat(3000), "è".repeat(500)),
    );
    let workspace = Workspace::open(&fixture.0).unwrap();
    let result = execute(&workspace, "search_text", r#"{"query":"needle"}"#);
    let found = &result.get("matches").and_then(Value::array).unwrap()[0];
    assert!(
        found
            .get("text")
            .and_then(Value::text)
            .unwrap()
            .contains("needle")
    );
    assert_eq!(
        found.get("byte_column").and_then(Value::unsigned),
        Some(9001)
    );
    assert_eq!(found.get("excerpt_truncated"), Some(&Value::Bool(true)));
}

#[test]
fn strict_arguments_and_schemas_cover_the_advertised_tools() {
    for (tool, args) in [
        ("run_command", r#"{}"#),
        ("read_file", r#"{}"#),
        ("read_file", r#"{"path":"text","start_line":0}"#),
        ("read_file", r#"{"path":"text","max_lines":401}"#),
        ("recall_receipts", r#"{"turn":0}"#),
        ("recall_receipts", r#"{"turn":0,"step":-1}"#),
        (
            "recall_receipts",
            r#"{"turn":0,"step":0,"session_id":"other"}"#,
        ),
        ("list_files", r#"{"path":null}"#),
        ("list_files", r#"{"limit":1.0}"#),
        ("list_files", r#"{"follow_links":true}"#),
        ("list_files", r#"{"path":""}"#),
        ("search_text", r#"{"query":""}"#),
        ("search_text", r#"{"query":"x","max_results":-1}"#),
        ("edit_file", r#"{"path":"file","old_text":"x"}"#),
        ("create_file", r#"{"path":"file","content":null}"#),
        (
            "create_file",
            r#"{"path":"file","content":"x","approved":true}"#,
        ),
    ] {
        assert!(Prepared::parse(tool, &json::parse(args, Default::default()).unwrap()).is_err());
    }
    assert_eq!(
        jecode::tools::definitions()
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        [
            "list_files",
            "read_file",
            "recall_receipts",
            "search_text",
            "create_file",
            "edit_file",
            "run_command"
        ]
    );
}

#[test]
fn image_selectors_accept_nullable_and_legacy_forms_without_repairing_bad_values() {
    let id = "a".repeat(64);
    for (argument, path, image_id) in [
        (
            r#"{"path":" sample.png","image_id":null}"#.to_owned(),
            Some(" sample.png"),
            None,
        ),
        (
            r#"{"path":"sample.png"}"#.to_owned(),
            Some("sample.png"),
            None,
        ),
        (
            format!(r#"{{"path":null,"image_id":"{id}"}}"#),
            None,
            Some(id.as_str()),
        ),
        (format!(r#"{{"image_id":"{id}"}}"#), None, Some(id.as_str())),
    ] {
        let args = json::parse(&argument, Default::default()).unwrap();
        match Prepared::parse("view_image", &args).unwrap() {
            Prepared::Image {
                path: actual_path,
                image_id: actual_id,
            } => {
                assert_eq!(actual_path.as_deref(), path);
                assert_eq!(actual_id.as_deref(), image_id);
            }
            _ => panic!("expected image selector"),
        }
    }
    for argument in [
        r#"{}"#.to_owned(),
        r#"{"path":null,"image_id":null}"#.to_owned(),
        format!(r#"{{"path":"sample.png","image_id":"{id}"}}"#),
        r#"{"path":"","image_id":null}"#.to_owned(),
        r#"{"path":" \t ","image_id":null}"#.to_owned(),
        r#"{"path":null,"image_id":""}"#.to_owned(),
        r#"{"path":null,"image_id":" \t "}"#.to_owned(),
        r#"{"path":null,"image_id":"ABC"}"#.to_owned(),
        r#"{"path":null,"image_id":"aaaa"}"#.to_owned(),
        format!(r#"{{"path":null,"image_id":"{}"}}"#, "A".repeat(64)),
        r#"{"path":42,"image_id":null}"#.to_owned(),
        r#"{"path":"sample.png","image_id":false}"#.to_owned(),
        r#"{"path":[],"image_id":"aaaa"}"#.to_owned(),
        r#"{"path":"sample.png","image_id":{}}"#.to_owned(),
        r#"{"path":"sample.png","image_id":null,"extra":1}"#.to_owned(),
    ] {
        let args = json::parse(&argument, Default::default()).unwrap();
        assert!(
            Prepared::parse("view_image", &args).is_err(),
            "accepted invalid selectors: {argument}"
        );
    }
}

#[test]
fn direct_dispatch_cannot_bypass_the_ordered_session_controller() {
    let fixture = Fixture::new();
    fixture.write("file", "old");
    let workspace = Workspace::open(&fixture.0).unwrap();
    for (name, args) in [
        (
            "edit_file",
            r#"{"path":"file","old_text":"old","new_text":"new"}"#,
        ),
        ("create_file", r#"{"path":"new","content":"new"}"#),
        ("run_command", r#"{"command":"echo direct bypass"}"#),
    ] {
        assert_eq!(
            execute(&workspace, name, args).get("ok"),
            Some(&Value::Bool(false))
        );
    }
    assert_eq!(
        std::fs::read_to_string(fixture.0.join("file")).unwrap(),
        "old"
    );
    assert!(!fixture.0.join("new").exists());
}
