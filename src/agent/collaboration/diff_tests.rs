use super::recovery_tests::{action, create};
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
};
use std::fs;

fn write(id: &str, path: &str, content: &str) -> Value {
    tool_call(
        id,
        "write",
        Value::object([
            ("path", Value::string(path)),
            ("content", Value::string(content)),
        ]),
    )
}

/// A returned shared collaborator that sent these calls, one response each.
fn returned(project: &Directory, home: &Directory, calls: Vec<Value>) -> (Agent, HttpFixture) {
    let mut responses: Vec<_> = calls
        .into_iter()
        .map(|call| (200, completion("", vec![call])))
        .collect();
    responses.push((200, completion("Done", vec![])));
    let fixture = HttpFixture::new(responses);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    parent.agent_tool(&create("shared")).unwrap();
    parent
        .collaboration()
        .wait(Some("a1"), Duration::from_secs(10), &parent.cancellation)
        .unwrap();
    (parent, fixture)
}

fn request(path: &str) -> Value {
    let mut request = action("diff");
    if let Value::Object(fields) = &mut request {
        fields.insert("path".into(), Value::string(path));
    }
    request
}

fn history(parent: &Agent, path: &str) -> String {
    parent
        .agent_tool(&request(path))
        .unwrap()
        .get("diff")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned()
}

#[test]
fn shared_work_diff_exposes_recorded_edits_without_claiming_a_baseline() {
    let project = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("notes.txt"), "before\n").unwrap();
    let (parent, fixture) = returned(
        &project,
        &home,
        vec![write("change", "notes.txt", "after\n")],
    );
    let summary = parent.agent_tool(&action("diff")).unwrap();
    assert_eq!(
        summary.get("source").and_then(Value::as_str),
        Some("recorded_tool_changes")
    );
    assert_eq!(
        summary.get("files").and_then(Value::as_array).unwrap(),
        &[Value::string("notes.txt")]
    );
    let mut request = request("notes.txt");
    if let Value::Object(fields) = &mut request {
        fields.insert("limit".into(), Value::number(24));
    }
    let page = parent.agent_tool(&request).unwrap();
    let mut text = page
        .get("diff")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let next = page.get("next_offset").and_then(Value::as_usize).unwrap();
    assert!(next > 0);
    if let Value::Object(fields) = &mut request {
        fields.insert("offset".into(), Value::number(next));
        fields.insert("limit".into(), Value::number(65_536));
    }
    fs::write(project.path().join("notes.txt"), "later main edit\n").unwrap();
    let rest = parent.agent_tool(&request).unwrap();
    text.push_str(rest.get("diff").and_then(Value::as_str).unwrap());
    assert!(text.contains("-before\n"), "{text}");
    assert!(text.contains("+after\n"), "{text}");
    assert!(!text.contains("later main edit"));
    assert!(rest.get("review_token").is_none());
    assert_eq!(rest.get("complete"), Some(&Value::Bool(false)));
    fixture.finish();
}

#[test]
fn a_reused_call_id_never_shows_another_files_hunks() {
    let project = Directory::new();
    let home = Directory::new();
    let (parent, fixture) = returned(
        &project,
        &home,
        vec![
            write("reused", "first.txt", "first\n"),
            write("reused", "second.txt", "second\n"),
        ],
    );
    for (path, other) in [("first.txt", "+second"), ("second.txt", "+first")] {
        let text = history(&parent, path);
        assert!(!text.contains(other), "{path}: {text}");
        assert!(text.contains("call ID was reused"), "{path}: {text}");
    }
    fixture.finish();
}

#[test]
fn a_reused_call_id_on_one_file_keeps_each_edit() {
    let project = Directory::new();
    let home = Directory::new();
    let (parent, fixture) = returned(
        &project,
        &home,
        vec![
            write("reused", "notes.txt", "one\n"),
            write("reused", "notes.txt", "two\n"),
        ],
    );
    let text = history(&parent, "notes.txt");
    assert!(text.contains("+one\n"), "{text}");
    assert!(text.contains("+two\n"), "{text}");
    assert!(!text.contains("call ID was reused"), "{text}");
    fixture.finish();
}

#[test]
fn steps_and_multi_file_edits_page_by_the_paths_they_named() {
    let project = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("b.txt"), "old b\n").unwrap();
    fs::write(project.path().join("c.txt"), "old c\n").unwrap();
    let replacement = |path: &str, old: &str, new: &str| {
        Value::object([
            ("path", Value::string(path)),
            (
                "edits",
                Value::Array(vec![Value::object([
                    ("old_text", Value::string(old)),
                    ("new_text", Value::string(new)),
                ])]),
            ),
        ])
    };
    let step = |tool: &str, arguments: Value| {
        Value::object([("tool", Value::string(tool)), ("arguments", arguments)])
    };
    let chain = tool_call(
        "chain",
        "steps",
        Value::object([(
            "steps",
            Value::Array(vec![
                step(
                    "write",
                    Value::object([
                        ("path", Value::string("a.txt")),
                        ("content", Value::string("new a\n")),
                    ]),
                ),
                step(
                    "edit",
                    Value::object([(
                        "files",
                        Value::Array(vec![
                            replacement("b.txt", "old b", "new b"),
                            replacement("c.txt", "old c", "new c"),
                        ]),
                    )]),
                ),
            ]),
        )]),
    );
    let (parent, fixture) = returned(&project, &home, vec![chain]);
    let summary = parent.agent_tool(&action("diff")).unwrap();
    assert_eq!(
        summary.get("files").and_then(Value::as_array).unwrap(),
        &["a.txt", "b.txt", "c.txt"].map(Value::string)
    );
    for (path, added) in [
        ("a.txt", "+new a"),
        ("b.txt", "+new b"),
        ("c.txt", "+new c"),
    ] {
        let text = history(&parent, path);
        assert!(text.contains(added), "{path}: {text}");
        assert_eq!(text.matches("\n+").count(), 1, "{path}: {text}");
    }
    fixture.finish();
}

#[test]
fn an_unrecorded_path_is_refused_as_in_isolated_work() {
    let project = Directory::new();
    let home = Directory::new();
    let (parent, fixture) = returned(&project, &home, vec![write("change", "notes.txt", "new\n")]);
    let error = parent.agent_tool(&request("other.txt")).unwrap_err();
    assert!(error.contains("no recorded"), "{error}");
    fixture.finish();
}

#[test]
fn hunks_name_their_lines_and_a_line_ending_change_reads_as_itself() {
    let project = Directory::new();
    let home = Directory::new();
    let lines: Vec<String> = (1..=8).map(|line| format!("line {line}\n")).collect();
    fs::write(project.path().join("long.txt"), lines.concat()).unwrap();
    fs::write(project.path().join("endings.txt"), "a\nb\n").unwrap();
    let (parent, fixture) = returned(
        &project,
        &home,
        vec![
            write(
                "middle",
                "long.txt",
                &lines.concat().replace("line 5\n", "changed\n"),
            ),
            write("created", "new.txt", "new\n"),
            write("endings", "endings.txt", "a\r\nb\r\n"),
        ],
    );
    let text = history(&parent, "long.txt");
    assert!(text.contains("@@ -2,7 +2,7 @@\n"), "{text}");
    let text = history(&parent, "new.txt");
    assert!(text.contains("@@ -0,0 +1,1 @@\n"), "{text}");
    let text = history(&parent, "endings.txt");
    assert!(text.contains("Line endings: LF → CRLF\n"), "{text}");
    assert!(!text.contains("not recorded"), "{text}");
    fixture.finish();
}
