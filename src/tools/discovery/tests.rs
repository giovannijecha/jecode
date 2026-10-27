use super::*;
use crate::test_support::Directory;
use std::fs;

fn fixture() -> (Directory, std::path::PathBuf) {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    (directory, root)
}

fn put(root: &Path, path: &str, content: impl AsRef<[u8]>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn run<const N: usize>(root: &Path, name: &str, fields: [(&str, Value); N]) -> Value {
    execute(root, name, &Value::object(fields), &Cancellation::default()).unwrap()
}

fn paths<'a>(value: &'a Value, key: &str) -> Vec<&'a str> {
    value
        .get(key)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry.get("path").unwrap().as_str().unwrap())
        .collect()
}

#[test]
fn list_pages_sorted_tree_without_losing_directories_or_files() {
    let (_directory, root) = fixture();
    for path in ["z.txt", "src/b.rs", "src/a.rs", "a.txt"] {
        put(&root, path, "source");
    }
    let first = run(
        &root,
        "list",
        [
            ("recursive", Value::Bool(true)),
            ("limit", Value::number(2)),
        ],
    );
    assert_eq!(paths(&first, "entries"), ["a.txt", "src"]);
    assert_eq!(first.get("next_offset"), Some(&Value::number(3)));
    assert_eq!(first.get("complete"), Some(&Value::Bool(false)));
    let second = run(
        &root,
        "list",
        [
            ("recursive", Value::Bool(true)),
            ("offset", Value::number(3)),
        ],
    );
    assert_eq!(paths(&second, "entries"), ["src/a.rs", "src/b.rs", "z.txt"]);
    assert_eq!(second.get("complete"), Some(&Value::Bool(true)));
    assert_eq!(second.get("next_offset"), Some(&Value::Null));
    let entries = second.get("entries").unwrap().as_array().unwrap();
    assert_eq!(entries[0].get("depth"), Some(&Value::number(2)));
    let shallow = run(&root, "list", []);
    assert_eq!(paths(&shallow, "entries"), ["a.txt", "src", "z.txt"]);
    let scoped = run(&root, "list", [("path", Value::string("src"))]);
    assert_eq!(paths(&scoped, "entries"), ["src/a.rs", "src/b.rs"]);
    assert_eq!(
        scoped.get("entries").unwrap().as_array().unwrap()[0].get("depth"),
        Some(&Value::number(1))
    );
}

#[test]
fn discovery_filters_hidden_ignored_files_and_directory_rules_independently() {
    let (_directory, root) = fixture();
    put(&root, ".git/info/exclude", "private.txt\n");
    put(&root, ".gitignore", "*.log\ncache/\nonly-dir/\n");
    put(&root, "src/.gitignore", "!kept.log\n");
    for path in [
        ".secret",
        "public.txt",
        "private.txt",
        "lost.log",
        "cache/data.txt",
        "src/kept.log",
        "src/lost.log",
        "only-dir",
    ] {
        put(&root, path, "needle");
    }
    let listed = run(&root, "list", [("recursive", Value::Bool(true))]);
    assert_eq!(
        paths(&listed, "entries"),
        ["only-dir", "public.txt", "src", "src/kept.log"]
    );
    assert!(
        listed
            .get("skipped")
            .unwrap()
            .get("hidden")
            .unwrap()
            .as_usize()
            .unwrap()
            > 0
    );
    let ignored = run(&root, "list", [("include_ignored", Value::Bool(true))]);
    assert!(paths(&ignored, "entries").contains(&"private.txt"));
    assert!(!paths(&ignored, "entries").contains(&".secret"));
    let hidden = run(&root, "list", [("include_hidden", Value::Bool(true))]);
    assert!(paths(&hidden, "entries").contains(&".secret"));
    assert!(!paths(&hidden, "entries").contains(&"private.txt"));
    assert!(!paths(&hidden, "entries").contains(&".git"));
    let explicit = run(
        &root,
        "search",
        [
            ("path", Value::string("private.txt")),
            ("query", Value::string("needle")),
        ],
    );
    assert_eq!(paths(&explicit, "matches"), ["private.txt"]);
    let hidden_dir = run(&root, "list", [("path", Value::string(".git"))]);
    assert_eq!(paths(&hidden_dir, "entries"), [".git/info"]);
}

#[test]
fn explicit_globs_are_workspace_relative_and_do_not_prune_unmatched_directories() {
    let (_directory, root) = fixture();
    put(&root, "src/nested/main.rs", "needle");
    put(&root, "src/other.txt", "needle");
    put(&root, "root.rs", "needle");
    let rust = run(
        &root,
        "list",
        [
            ("recursive", Value::Bool(true)),
            ("pattern", Value::string("*.rs")),
        ],
    );
    assert_eq!(paths(&rust, "entries"), ["root.rs", "src/nested/main.rs"]);
    let scoped = run(
        &root,
        "search",
        [
            ("path", Value::string("src")),
            ("pattern", Value::string("src/**/*.rs")),
            ("query", Value::string("needle")),
        ],
    );
    assert_eq!(paths(&scoped, "matches"), ["src/nested/main.rs"]);
    let empty = run(&root, "list", [("pattern", Value::string("*.none"))]);
    assert!(paths(&empty, "entries").is_empty());
    assert_eq!(empty.get("complete"), Some(&Value::Bool(true)));
}

#[test]
fn search_returns_line_context_exact_match_bytes_and_live_pagination() {
    let (_directory, root) = fixture();
    put(
        &root,
        "sample.txt",
        "before\r\nneedle one\r\nbridge\r\nneedle two\r\nafter\r\n",
    );
    let first = run(
        &root,
        "search",
        [
            ("query", Value::string("needle")),
            ("context", Value::number(1)),
            ("limit", Value::number(1)),
        ],
    );
    let matched = &first.get("matches").unwrap().as_array().unwrap()[0];
    assert_eq!(matched.get("line"), Some(&Value::number(2)));
    assert_eq!(matched.get("byte_offset"), Some(&Value::number(8)));
    assert_eq!(matched.get("text"), Some(&Value::string("needle one")));
    assert_eq!(matched.get("text_truncated"), Some(&Value::Bool(false)));
    assert_eq!(
        matched.get("before").unwrap().as_array().unwrap()[0].get("text"),
        Some(&Value::string("before"))
    );
    assert_eq!(
        matched.get("after").unwrap().as_array().unwrap()[0].get("line"),
        Some(&Value::number(3))
    );
    assert_eq!(first.get("next_offset"), Some(&Value::number(2)));
    let second = run(
        &root,
        "search",
        [
            ("query", Value::string("needle")),
            ("offset", Value::number(2)),
            ("context", Value::number(1)),
        ],
    );
    assert_eq!(second.get("matches_returned"), Some(&Value::number(1)));
    assert_eq!(
        second.get("matches").unwrap().as_array().unwrap()[0].get("line"),
        Some(&Value::number(4))
    );
    assert_eq!(second.get("complete"), Some(&Value::Bool(true)));
    assert_eq!(second.get("next_offset"), Some(&Value::Null));
}

#[test]
fn search_is_literal_ascii_case_optional_and_retains_unicode() {
    let (_directory, root) = fixture();
    put(&root, "source.txt", "a.*b\nA.*B\naXYZb\nà🙂 NEEDLE\n");
    let literal = run(&root, "search", [("query", Value::string("a.*b"))]);
    assert_eq!(literal.get("matches_returned"), Some(&Value::number(1)));
    let folded = run(
        &root,
        "search",
        [
            ("query", Value::string("a.*b")),
            ("case_sensitive", Value::Bool(false)),
            ("context", Value::number(0)),
        ],
    );
    assert_eq!(folded.get("matches_returned"), Some(&Value::number(2)));
    let unicode = run(
        &root,
        "search",
        [
            ("query", Value::string("à🙂 needle")),
            ("case_sensitive", Value::Bool(false)),
        ],
    );
    assert_eq!(unicode.get("matches_returned"), Some(&Value::number(1)));
    let missing = run(&root, "search", [("query", Value::string("absent"))]);
    assert_eq!(missing.get("matches_returned"), Some(&Value::number(0)));
    assert_eq!(missing.get("complete"), Some(&Value::Bool(true)));
}

#[test]
fn long_line_search_streams_across_utf8_and_chunk_boundaries_and_can_be_read_again() {
    let (_directory, root) = fixture();
    let source = format!(
        "{}🙂needle{}\n",
        "x".repeat(16 * 1024 - 1),
        "y".repeat(256 * 1024)
    );
    put(&root, "long.txt", &source);
    let result = run(&root, "search", [("query", Value::string("🙂needle"))]);
    assert!(result.encode().len() <= crate::tools::OUTPUT_LIMIT);
    let matched = &result.get("matches").unwrap().as_array().unwrap()[0];
    assert_eq!(
        matched.get("byte_offset"),
        Some(&Value::number(16 * 1024 - 1))
    );
    assert_eq!(matched.get("text_truncated"), Some(&Value::Bool(true)));
    assert!(
        matched
            .get("text")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("🙂needle")
    );
    let read = super::super::files::read_page(
        &root.join("long.txt"),
        &Value::object([("byte_offset", matched.get("byte_offset").unwrap().clone())]),
        &Cancellation::default(),
    )
    .unwrap();
    assert!(
        read.get("content")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("🙂needle")
    );
}

#[test]
fn search_pages_by_encoded_budget_and_does_not_hide_invalid_file_tails() {
    let (_directory, root) = fixture();
    let source = format!("needle{}\n", "x".repeat(1600)).repeat(150);
    put(&root, "a.txt", source);
    let first = run(
        &root,
        "search",
        [
            ("query", Value::string("needle")),
            ("context", Value::number(2)),
            ("limit", Value::number(1000)),
        ],
    );
    assert!(first.encode().len() <= crate::tools::OUTPUT_LIMIT);
    assert!(first.get("matches_returned").unwrap().as_usize().unwrap() < 150);
    assert!(first.get("next_offset").unwrap().as_usize().unwrap() > 1);
    put(&root, "binary.bin", b"\0needle");
    put(
        &root,
        "invalid.txt",
        b"needle valid\n\xffbad\nneedle missed\n",
    );
    let partial = run(
        &root,
        "search",
        [
            ("query", Value::string("needle")),
            ("path", Value::string("invalid.txt")),
        ],
    );
    assert_eq!(partial.get("matches_returned"), Some(&Value::number(1)));
    assert_eq!(partial.get("complete"), Some(&Value::Bool(false)));
    assert_eq!(partial.get("errors_count"), Some(&Value::number(1)));
    let binary = run(
        &root,
        "search",
        [
            ("query", Value::string("needle")),
            ("path", Value::string("binary.bin")),
        ],
    );
    assert_eq!(binary.get("matches_returned"), Some(&Value::number(0)));
    assert_eq!(
        binary.get("skipped").unwrap().get("binary"),
        Some(&Value::number(1))
    );
    assert_eq!(binary.get("complete"), Some(&Value::Bool(true)));
}

#[test]
fn invalid_arguments_and_cancellation_fail_without_writing() {
    let (_directory, root) = fixture();
    for (name, arguments) in [
        ("search", Value::object([("query", Value::string(""))])),
        (
            "search",
            Value::object([("query", Value::string("two\nlines"))]),
        ),
        (
            "search",
            Value::object([
                ("query", Value::string("x")),
                ("context", Value::number(11)),
            ]),
        ),
        ("list", Value::object([("limit", Value::number(2001))])),
        (
            "list",
            Value::object([("recursive", Value::string("true"))]),
        ),
        ("list", Value::object([("path", Value::string(".."))])),
        ("list", Value::object([("path", Value::string("missing"))])),
    ] {
        assert!(execute(&root, name, &arguments, &Cancellation::default()).is_err());
    }
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        execute(&root, "list", &Value::object([]), &cancel)
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(
        execute(
            &root,
            "search",
            &Value::object([("query", Value::string("needle"))]),
            &cancel
        )
        .unwrap_err()
        .contains("cancelled")
    );
    assert_eq!(fs::read_dir(root).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn recursive_discovery_lists_but_does_not_follow_symlinks_or_special_files() {
    use std::os::unix::fs::symlink;
    let (_directory, root) = fixture();
    let (_outside, outside) = fixture();
    put(&outside, "outside.txt", "needle");
    put(&root, "inside.txt", "needle");
    symlink(&outside, root.join("linked")).unwrap();
    symlink(&root, root.join("cycle")).unwrap();
    let list = run(&root, "list", [("recursive", Value::Bool(true))]);
    assert_eq!(paths(&list, "entries"), ["cycle", "inside.txt", "linked"]);
    let search = run(&root, "search", [("query", Value::string("needle"))]);
    assert_eq!(paths(&search, "matches"), ["inside.txt"]);
    assert_eq!(
        search.get("skipped").unwrap().get("links"),
        Some(&Value::number(2))
    );
}
