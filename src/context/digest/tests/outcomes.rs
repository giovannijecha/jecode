use super::*;

#[test]
fn no_op_edits_never_claim_a_file_changed() {
    let edit = Value::object([
        ("old_text", Value::string("same")),
        ("new_text", Value::string("same")),
    ]);
    let arguments = Value::object([
        ("path", Value::string("unchanged.js")),
        ("edits", Value::Array(vec![edit.clone()])),
    ]);
    let result = Value::object([("changed", Value::Bool(false))]);
    let cases = [
        (arguments.clone(), result.clone()),
        (
            Value::object([("files", Value::Array(vec![arguments]))]),
            Value::object([(
                "files",
                Value::Array(vec![Value::object([
                    ("path", Value::string("unchanged.js")),
                    ("changed", Value::Bool(false)),
                ])]),
            )]),
        ),
    ];
    for (arguments, result) in cases {
        let messages = vec![
            user("system"),
            user("Apply the replacement."),
            assistant("", &[("edit", "edit", arguments)]),
            tool("edit", result),
            assistant("Already identical.", &[]),
        ];
        for level in [1, 2, 3] {
            let text = render(
                &messages,
                &Epoch::new(1, messages.len(), level, String::new()),
                &BTreeSet::new(),
            );
            assert!(!text.contains("Changed:"), "{text}");
            if level == 1 {
                assert!(text.contains("unchanged.js (unchanged)"), "{text}");
            }
        }
    }
}

#[test]
fn partial_read_failures_keep_the_first_path_and_cause_when_condensed() {
    let messages = vec![
        user("system"),
        user("Read the project files."),
        assistant(
            "",
            &[(
                "read",
                "read",
                Value::object([(
                    "paths",
                    Value::Array(["present.js", "missing.js"].map(Value::string).into()),
                )]),
            )],
        ),
        tool(
            "read",
            Value::object([(
                "files",
                Value::Array(vec![
                    Value::object([
                        ("path", Value::string("present.js")),
                        ("content", Value::string("1: code\n")),
                        ("lines_returned", Value::number(1)),
                        ("total_lines", Value::number(1)),
                    ]),
                    Value::object([
                        ("path", Value::string("missing.js")),
                        ("error", Value::string("File does not exist")),
                    ]),
                ]),
            )]),
        ),
        assistant("One file could not be read.", &[]),
    ];
    for level in [1, 2, 3] {
        let text = render(
            &messages,
            &Epoch::new(1, messages.len(), level, String::new()),
            &BTreeSet::new(),
        );
        assert!(text.contains("missing.js"), "{text}");
        assert!(text.contains("File does not exist"), "{text}");
        if level > 1 {
            assert!(text.contains("First failed call [3]"), "{text}");
            assert!(text.contains("Failed calls: 1"), "{text}");
        }
    }
}

#[test]
fn large_partial_edits_keep_the_first_failed_file_before_successes() {
    let mut files = Vec::new();
    let mut receipts = Vec::new();
    for index in 0..12 {
        let path = format!("src/file{index}.js");
        files.push(Value::object([
            ("path", Value::string(&path)),
            (
                "edits",
                Value::Array(vec![Value::object([
                    ("old_text", Value::string("old")),
                    ("new_text", Value::string("replacement ".repeat(15))),
                ])]),
            ),
        ]));
        let mut receipt = Value::object([("path", Value::string(path))]);
        if let Value::Object(fields) = &mut receipt {
            fields.insert(
                if index < 9 { "changed" } else { "error" }.into(),
                if index < 9 {
                    Value::Bool(true)
                } else {
                    Value::string(format!(
                        "Missing exact match {index}: {}",
                        "detail ".repeat(60)
                    ))
                },
            );
        }
        receipts.push(receipt);
    }
    let messages = vec![
        user("system"),
        user("Apply the replacements."),
        assistant(
            "",
            &[(
                "edit",
                "edit",
                Value::object([("files", Value::Array(files))]),
            )],
        ),
        tool("edit", Value::object([("files", Value::Array(receipts))])),
    ];
    for level in [1, 2, 3] {
        let text = render(
            &messages,
            &Epoch::new(1, messages.len(), level, String::new()),
            &BTreeSet::new(),
        );
        assert!(
            text.contains("src/file9.js failed: Missing exact match 9"),
            "{text}"
        );
        if level > 1 {
            let changed = text
                .lines()
                .find(|line| line.starts_with("Changed:"))
                .unwrap();
            assert!(changed.contains("src/file8.js"), "{text}");
            assert!(!changed.contains("src/file9.js"), "{text}");
        }
    }
}

#[test]
fn long_multi_file_edit_keeps_every_changed_path_at_all_record_levels() {
    let mut files = Vec::new();
    let mut receipts = Vec::new();
    let mut paths = Vec::new();
    for index in 0..12 {
        let path = format!("src/format/section{index}/file{index}.js");
        paths.push(path.clone());
        files.push(Value::object([
            ("path", Value::string(&path)),
            (
                "edits",
                Value::Array(vec![Value::object([
                    ("old_text", Value::string("old")),
                    (
                        "new_text",
                        Value::string(format!("ANCHOR-{index}-{}", "x".repeat(100))),
                    ),
                ])]),
            ),
        ]));
        receipts.push(Value::object([
            ("path", Value::string(&path)),
            ("changed", Value::Bool(true)),
        ]));
    }
    let messages = vec![
        user("system"),
        user("Apply every replacement."),
        assistant(
            "",
            &[(
                "edit",
                "edit",
                Value::object([("files", Value::Array(files))]),
            )],
        ),
        tool("edit", Value::object([("files", Value::Array(receipts))])),
    ];
    for level in [1, 2, 3] {
        let text = render(
            &messages,
            &Epoch::new(1, messages.len(), level, String::new()),
            &BTreeSet::new(),
        );
        for path in &paths {
            assert!(text.contains(path), "level {level} missing {path}");
        }
    }
}

#[test]
fn failed_questions_keep_their_failure_in_every_record_level() {
    let messages = vec![
        user("system"),
        user("Ask for the missing choice."),
        assistant("", &[("question", "ask_user", Value::object([]))]),
        tool(
            "question",
            Value::object([("error", Value::string("Interaction is unavailable"))]),
        ),
    ];
    for level in [1, 2, 3] {
        let text = render(
            &messages,
            &Epoch::new(1, messages.len(), level, String::new()),
            &BTreeSet::new(),
        );
        assert!(text.contains("Interaction is unavailable"), "{text}");
        if level > 1 {
            assert!(text.contains("Failed calls: 1"), "{text}");
        }
    }
}
