use super::*;
use crate::test_support::Directory;

mod ambiguity;

fn batch(steps: &[(&str, &str)]) -> Value {
    Value::object([
        ("path", Value::string("source.txt")),
        (
            "edits",
            Value::Array(
                steps
                    .iter()
                    .map(|(old, new)| {
                        Value::object([
                            ("old_text", Value::string(*old)),
                            ("new_text", Value::string(*new)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

#[test]
fn batch_edits_use_unique_matches_in_staged_order_and_report_one_change() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    fs::write(root.join("source.txt"), "alpha\nbeta\n").unwrap();
    let (result, change) = edit_with_change(
        &root,
        &batch(&[("alpha", "gamma"), ("gamma\nbeta", "delta")]),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(result.get("bytes_written"), Some(&Value::number(6)));
    assert_eq!(result.get("changed"), Some(&Value::Bool(true)));
    assert_eq!(
        fs::read_to_string(root.join("source.txt")).unwrap(),
        "delta\n"
    );
    let change = change.unwrap();
    assert_eq!((change.added, change.removed), (1, 2));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn rejected_later_batch_step_keeps_original_bytes() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source.txt");
    let original = b"head\r\nababa\r\nend";
    for (steps, reason) in [
        (vec![("head", "new"), ("missing", "x")], "not found"),
        (vec![("head", "new"), ("ab", "x")], "more than once"),
        (vec![("head", "new"), ("aba", "x")], "more than once"),
    ] {
        fs::write(&path, original).unwrap();
        let (result, change) =
            edit_with_change(&root, &batch(&steps), &Cancellation::default()).unwrap();
        assert!(
            result
                .get("error")
                .and_then(Value::as_str)
                .unwrap()
                .contains(reason)
        );
        assert_eq!(result.get("outcome"), Some(&Value::string("not_started")));
        assert_eq!(
            result.get("current_content").and_then(Value::as_str),
            Some(std::str::from_utf8(original).unwrap())
        );
        assert!(change.is_none());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }
}

#[test]
fn later_batch_mismatches_explain_staged_matching_and_original_context() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source.txt");
    let original = b"alpha beta\n";
    for (steps, reason) in [
        (vec![("alpha", "gamma"), ("alpha beta", "x")], "not found"),
        (
            vec![("alpha", "gamma gamma"), ("gamma", "x")],
            "more than once",
        ),
    ] {
        fs::write(&path, original).unwrap();
        let rejected = edit(&root, &batch(&steps)).unwrap();
        let error = rejected.get("error").and_then(Value::as_str).unwrap();
        assert!(error.starts_with("edit 2: "), "{error}");
        assert!(error.contains(reason), "{error}");
        assert!(error.contains("staged by preceding edits"), "{error}");
        assert!(error.contains("no edits were saved"), "{error}");
        assert!(
            error.contains("current_content comes from the unchanged original file"),
            "{error}"
        );
        assert_eq!(
            rejected.get("current_content").and_then(Value::as_str),
            Some("alpha beta\n")
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}

#[test]
fn later_streaming_mismatches_explain_staged_matching_and_original_context() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source.txt");
    let original = format!("alpha beta\n{}", "x".repeat(MAX_DIFF_BYTES));
    for (steps, reason) in [
        (vec![("alpha", "gamma"), ("alpha beta", "x")], "not found"),
        (
            vec![("alpha", "gamma gamma"), ("gamma", "x")],
            "more than once",
        ),
    ] {
        fs::write(&path, &original).unwrap();
        let rejected = edit(&root, &batch(&steps)).unwrap();
        let error = rejected.get("error").and_then(Value::as_str).unwrap();
        assert!(error.starts_with("edit 2: "), "{error}");
        assert!(error.contains(reason), "{error}");
        assert!(error.contains("staged by preceding edits"), "{error}");
        assert!(error.contains("no edits were saved"), "{error}");
        assert!(
            error.contains("current_content comes from the unchanged original file"),
            "{error}"
        );
        assert!(
            rejected
                .get("current_content")
                .and_then(Value::as_str)
                .unwrap()
                .starts_with("alpha beta\n")
        );
        if reason == "more than once" {
            assert_eq!(
                rejected.get("match_context_source").and_then(Value::as_str),
                Some("staged")
            );
            let windows = rejected
                .get("match_context")
                .and_then(Value::as_array)
                .unwrap();
            assert_eq!(windows.len(), 2);
            assert!(
                windows[0]
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap()
                    .contains("gamma gamma beta")
            );
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }
}

#[test]
fn batch_rejects_invalid_shapes_and_noop_does_not_replace_file() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "original").unwrap();
    let retired = Value::object([
        ("path", Value::string("source.txt")),
        ("old_text", Value::string("original")),
        ("new_text", Value::string("changed")),
    ]);
    assert!(
        edit(&root, &retired)
            .unwrap_err()
            .contains("edits must be an array")
    );
    assert!(
        edit(&root, &batch(&[]))
            .unwrap_err()
            .contains("at least one")
    );
    assert!(edit(&root, &batch(&[("", "x")])).is_err());
    assert!(edit(&root, &batch(&[("original", "\0")])).is_err());
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let (result, change) = edit_with_change(
        &root,
        &batch(&[("original", "original")]),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(result.get("bytes_written"), Some(&Value::number(0)));
    assert!(change.is_none());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    assert_eq!(fs::read(&path).unwrap(), b"original");
}

#[test]
fn streaming_batch_composes_each_step_and_installs_only_after_validation() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source.txt");
    let original = format!("start:{}:end", "x".repeat(MAX_DIFF_BYTES));
    fs::write(&path, &original).unwrap();
    let rejected = edit(&root, &batch(&[("start", "begin"), ("absent", "x")])).unwrap();
    assert!(
        rejected
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("edit 2")
    );
    assert!(
        original.starts_with(
            rejected
                .get("current_content")
                .and_then(Value::as_str)
                .unwrap()
        )
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    let (result, change) = edit_with_change(
        &root,
        &batch(&[("start", "begin"), ("begin:", "front:"), (":end", ":tail")]),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        result.get("bytes_written"),
        Some(&Value::number(original.len() + 1))
    );
    assert_eq!(result.get("changed"), Some(&Value::Bool(true)));
    assert!(change.is_none());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("front:{}:tail", "x".repeat(MAX_DIFF_BYTES))
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    let (noop, change) = edit_with_change(
        &root,
        &batch(&[("front:", "front:")]),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(noop.get("bytes_written"), Some(&Value::number(0)));
    assert_eq!(noop.get("changed"), Some(&Value::Bool(false)));
    assert!(change.is_none());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn streaming_batch_matches_across_inserted_and_original_utf8_ranges() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source.txt");
    let filler = "x".repeat(MAX_DIFF_BYTES);
    let original = format!("αhead|middle|tail🦀{filler}end");
    fs::write(&path, original).unwrap();
    let steps = [
        ("head", "first-long"),
        ("αfirst", "Ω"),
        ("Ω-long|middle", "body"),
        ("tail🦀", ""),
        ("body|", "🦀\r\n"),
        ("end", "Z"),
    ];
    let result = edit(&root, &batch(&steps)).unwrap();
    let expected = format!("🦀\r\n{filler}Z");
    assert_eq!(
        result.get("bytes_written"),
        Some(&Value::number(expected.len()))
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn streaming_batch_ambiguity_created_across_pieces_keeps_original_and_permissions() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source.txt");
    let original = format!("ab|aca{}", "x".repeat(MAX_DIFF_BYTES));
    fs::write(&path, &original).unwrap();
    let permissions = fs::metadata(&path).unwrap().permissions();
    // An additional "aca" spans the inserted "c" and original bytes on both sides.
    let result = edit(&root, &batch(&[("b|", "c"), ("aca", "done")])).unwrap();
    assert!(
        result
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("more than once")
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(fs::metadata(&path).unwrap().permissions(), permissions);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}
