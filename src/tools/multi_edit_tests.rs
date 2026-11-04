//! Multi-file edits: each file applies all or nothing, the others are still
//! written, and the result names each file's outcome in call order.
use super::*;
use crate::test_support::Directory;

fn replace(old: &str, new: &str) -> Value {
    Value::object([
        ("old_text", Value::string(old)),
        ("new_text", Value::string(new)),
    ])
}

fn entry(path: &str, edits: Vec<Value>) -> Value {
    Value::object([
        ("path", Value::string(path)),
        ("edits", Value::Array(edits)),
    ])
}

fn files(entries: Vec<Value>) -> Value {
    Value::object([("files", Value::Array(entries))])
}

fn run(tools: &Tools, arguments: &Value, begun: Begun) -> Execution {
    tools.execute_begun(begun, "edit", &arguments.encode(), &Cancellation::default())
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn a_failed_file_stays_unchanged_while_the_others_are_written() {
    let root = Directory::new();
    let write = |name: &str, content: &str| fs::write(root.path().join(name), content).unwrap();
    write("a.rs", "let a = 1;\nlet b = 2;\n");
    write("b.rs", "fn b() {}\n");
    write("c.rs", "x\n");
    let tools = Tools::new(root.path()).unwrap();
    let arguments = files(vec![
        entry(
            "a.rs",
            vec![replace("a = 1", "a = 10"), replace("b = 2", "b = 20")],
        ),
        // The first replacement matches; the second does not, so b.rs keeps
        // its bytes and the call still writes c.rs.
        entry(
            "b.rs",
            vec![replace("fn b", "fn bee"), replace("missing", "x")],
        ),
        entry("c.rs", vec![replace("x", "y\nz")]),
    ]);
    let execution = run(&tools, &arguments, Begun::default());
    let entries = execution
        .result
        .get("files")
        .and_then(Value::as_array)
        .unwrap();
    let paths: Vec<_> = entries.iter().map(|entry| text(entry, "path")).collect();
    assert_eq!(paths, ["a.rs", "b.rs", "c.rs"]);
    assert!(entries[0].get("bytes_written").and_then(Value::as_usize) > Some(0));
    assert!(text(&entries[1], "error").starts_with("edit 2: old text was not found"));
    assert_eq!(text(&entries[1], "current_content"), "fn b() {}\n");
    assert!(entries[2].get("error").is_none());
    let read = |name: &str| fs::read_to_string(root.path().join(name)).unwrap();
    assert_eq!(read("a.rs"), "let a = 10;\nlet b = 20;\n");
    assert_eq!(read("b.rs"), "fn b() {}\n");
    assert_eq!(read("c.rs"), "y\nz\n");
    let change = execution.change.unwrap();
    let changed: Vec<_> = change.files.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(changed, ["a.rs", "c.rs"]);
    assert_eq!((change.added, change.removed), (4, 3));
    assert!(change.hunks.is_empty());
    assert_eq!(
        crate::events::tool_summary("edit", &execution.result),
        "changed 2 files; 1 failed"
    );
}

#[test]
fn malformed_calls_are_refused_before_any_file_changes() {
    let root = Directory::new();
    fs::write(root.path().join("a.rs"), "one\n").unwrap();
    fs::write(root.path().join("b.rs"), "two\n").unwrap();
    let tools = Tools::new(root.path()).unwrap();
    let a = || entry("a.rs", vec![replace("one", "1")]);
    let many: Vec<_> = (0..=edit::MAX_FILES)
        .map(|index| entry(&format!("f{index}.rs"), vec![replace("x", "y")]))
        .collect();
    let mut mixed = files(vec![a()]);
    if let Value::Object(fields) = &mut mixed {
        fields.insert("path".into(), Value::string("b.rs"));
        fields.insert("edits".into(), Value::Array(vec![replace("two", "2")]));
    }
    for (arguments, expected) in [
        (files(vec![]), "1-12 entries"),
        (files(many), "1-12 entries"),
        (files(vec![a(), a()]), "names a.rs more than once"),
        (
            files(vec![a(), entry("./a.rs", vec![replace("one", "2")])]),
            "names one file twice",
        ),
        (mixed, "not both"),
        (
            files(vec![a(), Value::object([("edits", Value::Array(vec![]))])]),
            "path string",
        ),
        (
            Value::object([("files", Value::string("a.rs"))]),
            "must be an array",
        ),
    ] {
        let execution = run(&tools, &arguments, Begun::default());
        let error = text(&execution.result, "error");
        assert!(error.contains(expected), "{expected}: {error}");
        assert!(execution.change.is_none());
    }
    assert_eq!(
        fs::read_to_string(root.path().join("a.rs")).unwrap(),
        "one\n"
    );
    // Empty fields a native call fills in leave the single-file route alone.
    let single = Value::object([
        ("path", Value::string("b.rs")),
        ("edits", Value::Array(vec![replace("two", "2")])),
        ("files", Value::Array(vec![])),
    ]);
    assert!(
        run(&tools, &single, Begun::default())
            .result
            .get("bytes_written")
            .is_some()
    );
    // An entry's own edits fail with that file only.
    let execution = run(
        &tools,
        &files(vec![entry("b.rs", vec![]), a()]),
        Begun::default(),
    );
    let entries = execution
        .result
        .get("files")
        .and_then(Value::as_array)
        .unwrap();
    assert!(text(&entries[0], "error").contains("at least one replacement"));
    assert_eq!(fs::read_to_string(root.path().join("a.rs")).unwrap(), "1\n");
}

#[test]
fn every_written_file_gets_its_own_checkpoint_and_outside_files_need_the_calls_approval() {
    let home = Directory::new();
    let root = Directory::new();
    let other = Directory::new();
    fs::write(root.path().join("a.rs"), "one\n").unwrap();
    fs::write(root.path().join("b.rs"), "two\n").unwrap();
    let outside = other.path().join("c.rs");
    fs::write(&outside, "three\n").unwrap();
    let mut tools = Tools::new(root.path()).unwrap();
    let session = Session::offline(home.path(), tools.root());
    session.begin("conversation-a");
    tools.configure_checkpoints(session);
    let arguments = files(vec![
        entry("a.rs", vec![replace("one", "1")]),
        entry("b.rs", vec![replace("two", "2")]),
        entry(&outside.to_string_lossy(), vec![replace("three", "3")]),
    ]);
    // Without an approved outside access the outside file fails alone.
    let execution = run(&tools, &arguments, Begun::default());
    let entries = execution
        .result
        .get("files")
        .and_then(Value::as_array)
        .unwrap();
    let references: Vec<_> = entries[..2]
        .iter()
        .map(|entry| text(entry.get("checkpoint").unwrap(), "ref"))
        .collect();
    assert!(
        references
            .iter()
            .all(|reference| reference.starts_with("rc-"))
    );
    assert_ne!(references[0], references[1]);
    assert!(text(&entries[2], "error").contains("restricted to the working directory"));
    assert_eq!(fs::read_to_string(&outside).unwrap(), "three\n");

    // The call's one access names every file and is outside; approved, the
    // outside file is written from its own folder.
    let arguments = files(vec![
        entry("a.rs", vec![replace("1", "one")]),
        entry(&outside.to_string_lossy(), vec![replace("three", "3")]),
    ]);
    let access = tools.access("edit", &arguments.encode()).unwrap();
    assert!(access.outside);
    assert!(access.target.contains("a.rs, "));
    assert!(access.target.ends_with("c.rs"));
    let begun = Begun::default().with_permission(crate::permissions::Level::Full, Some(access));
    let execution = run(&tools, &arguments, begun);
    let entries = execution
        .result
        .get("files")
        .and_then(Value::as_array)
        .unwrap();
    assert!(entries.iter().all(|entry| entry.get("error").is_none()));
    assert_eq!(fs::read_to_string(&outside).unwrap(), "3\n");
    assert_eq!(
        text(entries[1].get("checkpoint").unwrap(), "reason"),
        "the file is outside the working folder"
    );
}

#[test]
fn a_path_without_replacements_beside_files_does_not_block_the_call() {
    // Claude once sent files as JSON text plus a stray path; both are accepted.
    let root = Directory::new();
    fs::write(root.path().join("a.rs"), "one\n").unwrap();
    fs::write(root.path().join("b.rs"), "two\n").unwrap();
    let tools = Tools::new(root.path()).unwrap();
    let listed = Value::Array(vec![
        entry("a.rs", vec![replace("one", "1")]),
        entry("b.rs", vec![replace("two", "2")]),
    ]);
    let sent = Value::object([
        ("files", Value::string(listed.encode())),
        ("path", Value::string("b.rs")),
    ])
    .encode();
    let decoded = crate::tools::coerce::arguments("edit", &sent).unwrap();
    let execution = run(&tools, &json::parse(&decoded).unwrap(), Begun::default());
    let entries = execution.result.get("files").and_then(Value::as_array);
    assert_eq!(
        entries.map(<[Value]>::len),
        Some(2),
        "{:?}",
        execution.result
    );
    assert_eq!(fs::read_to_string(root.path().join("a.rs")).unwrap(), "1\n");
    assert_eq!(fs::read_to_string(root.path().join("b.rs")).unwrap(), "2\n");
}
