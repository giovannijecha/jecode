use super::*;
use crate::test_support::Directory;
use crate::tools::OUTPUT_LIMIT;

fn read(root: &Path, arguments: &Value) -> Result<Value, String> {
    super::read(root, arguments, &crate::cancel::Cancellation::default())
}

fn arguments(entries: &[(&str, &str)]) -> Value {
    Value::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).into(), Value::string(*value)))
            .collect(),
    )
}

#[test]
fn reads_ranges_with_original_line_endings_and_replaces_atomically() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    write(
        &root,
        &arguments(&[
            ("path", "source.txt"),
            ("content", "first\r\nsecond\r\nthird"),
        ]),
    )
    .unwrap();
    let result = read(
        &root,
        &Value::object([
            ("path", Value::string("source.txt")),
            ("offset", Value::number(2)),
            ("limit", Value::number(1)),
        ]),
    )
    .unwrap();
    assert_eq!(
        result.get("content").unwrap().as_str(),
        Some("2: second\r\n")
    );
    assert_eq!(result.get("next_offset").unwrap().as_usize(), Some(3));
    edit(
        &root,
        &arguments(&[
            ("path", "source.txt"),
            ("old_text", "first\r\nsecond"),
            ("new_text", "updated"),
        ]),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("source.txt")).unwrap(),
        "updated\r\nthird"
    );
    write(
        &root,
        &arguments(&[("path", "source.txt"), ("content", "replacement")]),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("source.txt")).unwrap(),
        "replacement"
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn ambiguous_and_missing_edits_preserve_the_original_file() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    fs::write(root.join("source.txt"), "aaaa").unwrap();
    for old in ["aa", "missing", ""] {
        let failure = edit(
            &root,
            &arguments(&[
                ("path", "source.txt"),
                ("old_text", old),
                ("new_text", "changed"),
            ]),
        );
        assert!(failure.is_err() || failure.as_ref().unwrap().get("error").is_some());
        if let Ok(result) = failure {
            assert_eq!(
                result.get("outcome").and_then(Value::as_str),
                Some("not_started")
            );
            assert_eq!(
                result.get("current_content").and_then(Value::as_str),
                Some("aaaa")
            );
        }
        assert_eq!(fs::read_to_string(root.join("source.txt")).unwrap(), "aaaa");
    }
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn rejected_edits_return_bounded_literal_utf8_context_without_touching_the_file() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let original = format!("HEAD\r\n{}\r\nTAIL", "Ω-日本\"\\".repeat(3000));
    fs::write(root.join("source.txt"), &original).unwrap();
    let result = edit(
        &root,
        &arguments(&[
            ("path", "source.txt"),
            ("old_text", "missing"),
            ("new_text", "changed"),
        ]),
    )
    .unwrap();
    assert!(result.get("error").is_some());
    assert_eq!(result.get("context_complete"), Some(&Value::Bool(false)));
    let head = result
        .get("current_content")
        .and_then(Value::as_str)
        .unwrap();
    let tail = result.get("current_tail").and_then(Value::as_str).unwrap();
    let offset = result
        .get("current_tail_byte_offset")
        .and_then(Value::as_usize)
        .unwrap();
    assert!(original.starts_with(head));
    assert_eq!(&original[offset..], tail);
    assert!(head.len() + tail.len() <= 8192);
    assert_eq!(
        fs::read_to_string(root.join("source.txt")).unwrap(),
        original
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn file_tools_cannot_traverse_outside_the_root() {
    let directory = Directory::new();
    let workspace = directory.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(directory.path().join("outside.txt"), "untouched").unwrap();
    let root = fs::canonicalize(workspace).unwrap();
    let outside = directory
        .path()
        .join("outside.txt")
        .to_string_lossy()
        .into_owned();
    for path in ["../outside.txt", outside.as_str()] {
        assert!(read(&root, &arguments(&[("path", path)])).is_err());
        assert!(write(&root, &arguments(&[("path", path), ("content", "changed")])).is_err());
        assert!(
            edit(
                &root,
                &arguments(&[
                    ("path", path),
                    ("old_text", "untouched"),
                    ("new_text", "changed")
                ])
            )
            .is_err()
        );
    }
    assert!(
        write(
            &root,
            &arguments(&[("path", "../new.txt"), ("content", "changed")])
        )
        .is_err()
    );
    assert!(!directory.path().join("new.txt").exists());
    assert_eq!(
        fs::read_to_string(directory.path().join("outside.txt")).unwrap(),
        "untouched"
    );
}

#[test]
fn invalid_text_is_rejected_and_large_files_are_paged_without_losing_long_lines() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    for (name, bytes) in [("binary", vec![0, 1]), ("invalid", vec![0xff])] {
        fs::write(root.join(name), bytes).unwrap();
        assert!(read(&root, &arguments(&[("path", name)])).is_err());
    }
    let content = "🦀".repeat(300_000);
    write(
        &root,
        &arguments(&[("path", "source"), ("content", &content)]),
    )
    .unwrap();
    let mut byte_offset = 0;
    let mut restored = String::new();
    loop {
        let page = read(
            &root,
            &Value::object([
                ("path", Value::string("source")),
                ("byte_offset", Value::number(byte_offset)),
            ]),
        )
        .unwrap();
        let text = page.get("content").and_then(Value::as_str).unwrap();
        assert!(text.len() <= OUTPUT_LIMIT);
        restored.push_str(text);
        if let Some(next) = page.get("next_byte_offset").and_then(Value::as_usize) {
            assert!(next > byte_offset);
            byte_offset = next;
        } else {
            break;
        }
    }
    assert_eq!(restored, content);
    let eof = read(
        &root,
        &Value::object([
            ("path", Value::string("source")),
            ("byte_offset", Value::number(content.len())),
        ]),
    )
    .unwrap();
    assert_eq!(eof.get("bytes_returned"), Some(&Value::number(0)));
    assert_eq!(eof.get("next_byte_offset"), Some(&Value::Null));
    let editable = format!("{content}\nunique-tail");
    write(
        &root,
        &arguments(&[("path", "large-edit"), ("content", &editable)]),
    )
    .unwrap();
    edit(
        &root,
        &arguments(&[
            ("path", "large-edit"),
            ("old_text", "unique-tail"),
            ("new_text", "updated-tail"),
        ]),
    )
    .unwrap();
    assert!(
        fs::read_to_string(root.join("large-edit"))
            .unwrap()
            .ends_with("updated-tail")
    );
    fs::write(root.join("empty"), "").unwrap();
    let result = read(&root, &arguments(&[("path", "empty")])).unwrap();
    assert_eq!(result.get("content").unwrap().as_str(), Some(""));
    assert_eq!(result.get("next_offset"), Some(&Value::Null));
    assert!(
        read(
            &root,
            &Value::object([
                ("path", Value::string("source")),
                ("offset", Value::number(0))
            ])
        )
        .is_err()
    );
    assert!(read(&root, &arguments(&[("path", ".")])).is_err());
    assert!(
        write(
            &root,
            &arguments(&[("path", "missing/child"), ("content", "x")])
        )
        .is_err()
    );
}

#[test]
fn read_only_files_are_preserved_without_leaving_temporary_files() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source");
    fs::write(&path, "unchanged").unwrap();
    let original_permissions = fs::metadata(&path).unwrap().permissions();
    let mut read_only = original_permissions.clone();
    read_only.set_readonly(true);
    fs::set_permissions(&path, read_only).unwrap();
    let result = write(
        &root,
        &arguments(&[("path", "source"), ("content", "changed")]),
    );
    fs::set_permissions(&path, original_permissions).unwrap();
    assert!(result.unwrap_err().contains("read-only"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "unchanged");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn symbolic_links_cannot_escape_the_file_tool_root() {
    use std::os::unix::fs::symlink;
    let directory = Directory::new();
    let workspace = directory.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(directory.path().join("outside.txt"), "untouched").unwrap();
    symlink(directory.path().join("outside.txt"), workspace.join("link")).unwrap();
    let root = fs::canonicalize(workspace).unwrap();
    assert!(read(&root, &arguments(&[("path", "link")])).is_err());
    assert!(
        write(
            &root,
            &arguments(&[("path", "link"), ("content", "changed")])
        )
        .is_err()
    );
}
