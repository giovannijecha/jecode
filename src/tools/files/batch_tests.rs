use super::*;
use crate::checkpoints::session::Session;
use crate::test_support::Directory;

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
        None,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(result.get("bytes_written"), Some(&Value::number(6)));
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
            edit_with_change(&root, &batch(&steps), None, &Cancellation::default()).unwrap();
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
fn batch_rejects_invalid_shapes_and_noop_does_not_replace_file() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "original").unwrap();
    let mixed = Value::object([
        ("path", Value::string("source.txt")),
        ("old_text", Value::string("original")),
        ("new_text", Value::string("changed")),
        ("edits", Value::Array(vec![])),
    ]);
    assert!(edit(&root, &mixed).unwrap_err().contains("either"));
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
        None,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(result.get("bytes_written"), Some(&Value::number(0)));
    assert!(change.is_none());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    assert_eq!(fs::read(&path).unwrap(), b"original");
}

#[test]
fn streaming_batch_stages_each_step_and_installs_only_after_validation() {
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
        None,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        result.get("bytes_written"),
        Some(&Value::number(original.len() + 1))
    );
    assert!(change.is_none());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("front:{}:tail", "x".repeat(MAX_DIFF_BYTES))
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    let (noop, change) = edit_with_change(
        &root,
        &batch(&[("front:", "front:")]),
        None,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(noop.get("bytes_written"), Some(&Value::number(0)));
    assert!(change.is_none());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn batch_captures_one_checkpoint_only_after_all_matches_validate() {
    let home = Directory::new();
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "first second").unwrap();
    let session = Session::offline(home.path(), &root);
    session.begin("batch-edits");
    let (rejected, _) = edit_with_change(
        &root,
        &batch(&[("first", "changed"), ("missing", "x")]),
        Some(&session),
        &Cancellation::default(),
    )
    .unwrap();
    assert!(rejected.get("checkpoint").is_none());
    assert!(
        session
            .store()
            .unwrap()
            .list(10, &Cancellation::default())
            .unwrap()
            .is_empty()
    );
    let (result, _) = edit_with_change(
        &root,
        &batch(&[("first", "changed"), ("second", "other")]),
        Some(&session),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        result.get("checkpoint").unwrap().get("status"),
        Some(&Value::string("saved"))
    );
    assert_eq!(
        session
            .store()
            .unwrap()
            .list(10, &Cancellation::default())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "changed other");
}
