use super::*;
use crate::{cancel::Cancellation, json::Value, redact::Redactor, test_support::Directory};
use std::{collections::BTreeSet, fs, path::Path, thread};

fn new_map(project: &Directory, home: &Directory) -> Map {
    let mut map = Map::new(fs::canonicalize(project.path()).unwrap());
    map.configure(home.path());
    map
}

fn call(map: &Map, arguments: Value) -> Value {
    map.execute(&arguments, &Cancellation::default()).unwrap()
}

fn overview(map: &Map, arguments: Value) -> Value {
    let value = call(map, arguments);
    assert!(value.get("entries").and_then(Value::as_array).is_some());
    value
}

fn entries(value: &Value) -> &[Value] {
    value.get("entries").unwrap().as_array().unwrap()
}

fn note<'a>(value: &'a Value, id: &str) -> &'a Value {
    entries(value)
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
        .unwrap()
}

fn source(path: &str, line: Option<usize>) -> Value {
    let mut source = Value::object([("path", Value::string(path))]);
    if let (Some(line), Value::Object(fields)) = (line, &mut source) {
        fields.insert("line".into(), Value::number(line));
    }
    source
}

fn remember(map: &Map, title: &str, summary: &str, path: &str, line: Option<usize>) -> String {
    let value = call(
        map,
        Value::object([
            ("action", Value::string("remember")),
            ("title", Value::string(title)),
            ("summary", Value::string(summary)),
            ("sources", Value::Array(vec![source(path, line)])),
        ]),
    );
    assert_eq!(value.get("saved"), Some(&Value::Bool(true)));
    value.get("id").and_then(Value::as_str).unwrap().to_owned()
}

fn write(project: &Directory, path: &str, content: &str) {
    let target = project.path().join(path);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, content).unwrap();
}

#[test]
fn empty_overview_has_no_startup_storage_or_saved_notes() {
    let project = Directory::new();
    let home = Directory::new();
    let map = new_map(&project, &home);
    assert!(!home.path().join("projects").exists());
    let result = overview(&map, Value::object([]));
    assert_eq!(result.get("notes_count"), Some(&Value::number(0)));
    assert!(!home.path().join("projects").exists());
}

#[test]
fn full_file_fingerprint_detects_same_size_edit_even_with_restored_mtime() {
    let project = Directory::new();
    let home = Directory::new();
    write(&project, "src/main.rs", "fn main() { alpha(); }\n");
    let map = new_map(&project, &home);
    let id = remember(
        &map,
        "Entry",
        "This file starts the program",
        "src/main.rs",
        Some(1),
    );
    let current = overview(&map, Value::object([]));
    assert_eq!(
        note(&current, &id).get("freshness").and_then(Value::as_str),
        Some("current")
    );
    let path = project.path().join("src/main.rs");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "fn main() { omega(); }\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let changed = overview(&map, Value::object([]));
    let entry = note(&changed, &id);
    assert_eq!(
        entry.get("freshness").and_then(Value::as_str),
        Some("changed")
    );
    assert_eq!(
        entry.get("sources").unwrap().as_array().unwrap()[0]
            .get("status")
            .and_then(Value::as_str),
        Some("changed")
    );
    fs::rename(&path, project.path().join("src/renamed.rs")).unwrap();
    let missing = overview(&map, Value::object([]));
    assert_eq!(
        note(&missing, &id).get("freshness").and_then(Value::as_str),
        Some("missing")
    );
    fs::File::create(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let unverified = overview(&map, Value::object([]));
    assert_eq!(
        note(&unverified, &id)
            .get("freshness")
            .and_then(Value::as_str),
        Some("unverified")
    );
}

#[test]
fn notes_are_shared_across_instances_but_isolated_by_project_and_forget() {
    let project = Directory::new();
    let other_project = Directory::new();
    let home = Directory::new();
    write(&project, "src/main.rs", "fn main() {}\n");
    write(&other_project, "src/main.rs", "fn main() {}\n");
    let first = new_map(&project, &home);
    let id = remember(&first, "Shared", "Project entry", "src/main.rs", None);
    let second = new_map(&project, &home);
    let shared = overview(&second, Value::object([]));
    assert_eq!(
        note(&shared, &id).get("freshness").and_then(Value::as_str),
        Some("current")
    );
    let isolated = overview(&new_map(&other_project, &home), Value::object([]));
    assert_eq!(isolated.get("notes_count"), Some(&Value::number(0)));
    let removed = call(
        &second,
        Value::object([
            ("action", Value::string("forget")),
            ("id", Value::string(&id)),
        ]),
    );
    assert_eq!(removed.get("removed"), Some(&Value::Bool(true)));
    let fresh = overview(&new_map(&project, &home), Value::object([]));
    assert_eq!(fresh.get("notes_count"), Some(&Value::number(0)));
}

#[test]
fn concurrent_map_instances_preserve_distinct_notes() {
    let project = Directory::new();
    let home = Directory::new();
    write(&project, "src/main.rs", "fn main() {}\n");
    thread::scope(|scope| {
        for index in 0..6 {
            let project = &project;
            let home = &home;
            scope.spawn(move || {
                let map = new_map(project, home);
                remember(
                    &map,
                    &format!("Note {index}"),
                    "Shared source",
                    "src/main.rs",
                    None,
                );
            });
        }
    });
    let result = overview(&new_map(&project, &home), Value::object([]));
    assert_eq!(result.get("notes_count"), Some(&Value::number(6)));
    let titles: BTreeSet<_> = entries(&result)
        .iter()
        .filter(|entry| entry.get("kind").and_then(Value::as_str) == Some("note"))
        .filter_map(|entry| entry.get("title").and_then(Value::as_str))
        .collect();
    assert_eq!(titles.len(), 6);
}

#[test]
fn cancellation_before_remember_does_not_persist_a_note() {
    let project = Directory::new();
    let home = Directory::new();
    write(&project, "src/main.rs", "fn main() {}\n");
    let map = new_map(&project, &home);
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let result = map.execute(
        &Value::object([
            ("action", Value::string("remember")),
            ("title", Value::string("Cancelled")),
            ("summary", Value::string("Must not persist")),
            ("sources", Value::Array(vec![source("src/main.rs", None)])),
        ]),
        &cancellation,
    );
    assert!(result.is_err());
    assert_eq!(
        overview(&new_map(&project, &home), Value::object([])).get("notes_count"),
        Some(&Value::number(0))
    );
}

fn persisted_map(home: &Path) -> String {
    fs::read_to_string(map_path(home)).unwrap()
}

fn map_path(home: &Path) -> std::path::PathBuf {
    let projects = fs::read_dir(home.join("projects")).unwrap();
    let buckets: Vec<_> = projects.map(|entry| entry.unwrap().path()).collect();
    assert_eq!(buckets.len(), 1);
    buckets[0].join("map.json")
}

#[test]
fn note_fields_and_source_excerpt_are_redacted_before_storage_and_after_restart() {
    let project = Directory::new();
    let home = Directory::new();
    let secret = "fixture-secret-739";
    write(
        &project,
        "src/main.rs",
        &format!("// {secret}\nfn main() {{}}\n"),
    );
    let mut first = new_map(&project, &home);
    first.include_credentials(Redactor::new(secret.into()));
    let id = remember(
        &first,
        &format!("Title {secret}"),
        &format!("Summary {secret}"),
        "src/main.rs",
        Some(1),
    );
    assert!(!persisted_map(home.path()).contains(secret));
    let restarted = new_map(&project, &home);
    let result = overview(&restarted, Value::object([]));
    let entry = note(&result, &id);
    assert!(!entry.encode().contains(secret));
    assert!(entry.encode().contains("[redacted]"));
}

#[test]
fn paged_query_returns_each_matching_note_once_and_scopes_paths() {
    let project = Directory::new();
    let home = Directory::new();
    write(&project, "src/main.rs", "fn main() {}\n");
    write(&project, "docs/notes.md", "# Notes\n");
    let map = new_map(&project, &home);
    let mut expected = BTreeSet::new();
    for index in 0..15 {
        expected.insert(remember(
            &map,
            &format!("Atlas note {index}"),
            "Indexed map note",
            "src/main.rs",
            None,
        ));
    }
    let unrelated = remember(
        &map,
        "Atlas docs",
        "Not in source scope",
        "docs/notes.md",
        None,
    );
    let mut seen = BTreeSet::new();
    let mut offset = 1usize;
    loop {
        let page = overview(
            &map,
            Value::object([
                ("path", Value::string("src")),
                ("query", Value::string("ATLAS")),
                ("offset", Value::number(offset)),
                ("limit", Value::number(4)),
            ]),
        );
        for entry in entries(&page) {
            let id = entry.get("id").and_then(Value::as_str).unwrap();
            assert!(seen.insert(id.to_owned()), "duplicate note {id}");
        }
        match page.get("next_offset").and_then(Value::as_usize) {
            Some(next) => offset = next,
            None => break,
        }
    }
    assert_eq!(seen, expected);
    assert!(!seen.contains(&unrelated));
}

#[test]
fn unreadable_notes_leave_structure_available_and_refuse_to_replace_corrupt_data() {
    let project = Directory::new();
    let home = Directory::new();
    write(&project, "src/main.rs", "fn main() {}\n");
    let map = new_map(&project, &home);
    remember(&map, "Entry", "Known entry", "src/main.rs", None);
    let damaged = "{not valid json";
    fs::write(map_path(home.path()), damaged).unwrap();
    let result = overview(&map, Value::object([]));
    assert_eq!(result.get("notes_available"), Some(&Value::Bool(false)));
    assert_eq!(result.get("notes_count"), Some(&Value::Null));
    assert_eq!(result.get("complete"), Some(&Value::Bool(false)));
    assert!(
        result
            .get("errors_count")
            .and_then(Value::as_usize)
            .unwrap()
            >= 1
    );
    assert!(entries(&result).iter().any(|entry| {
        matches!(
            entry.get("origin").and_then(Value::as_str),
            Some("filesystem" | "source")
        )
    }));
    let attempted = map.execute(
        &Value::object([
            ("action", Value::string("remember")),
            ("title", Value::string("Another note")),
            (
                "summary",
                Value::string("Must leave damaged data unchanged"),
            ),
            ("sources", Value::Array(vec![source("src/main.rs", None)])),
        ]),
        &Cancellation::default(),
    );
    assert!(attempted.is_err());
    assert_eq!(persisted_map(home.path()), damaged);
}

#[test]
fn note_filters_follow_hidden_ignore_pattern_and_explicit_path_rules() {
    let project = Directory::new();
    let home = Directory::new();
    write(&project, "visible.rs", "pub fn visible() {}\n");
    write(&project, ".hidden.rs", "pub fn hidden() {}\n");
    write(&project, "ignored.rs", "pub fn ignored() {}\n");
    write(&project, ".gitignore", "ignored.rs\n");
    let map = new_map(&project, &home);
    let visible = remember(&map, "Visible", "Public source", "visible.rs", None);
    let hidden = remember(&map, "Hidden", "Dot-prefixed source", ".hidden.rs", None);
    let ignored = remember(&map, "Ignored", "Ignored source", "ignored.rs", None);
    let has = |result: &Value, id: &str| {
        entries(result)
            .iter()
            .any(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
    };
    let default = overview(&map, Value::object([]));
    assert!(has(&default, &visible));
    assert!(!has(&default, &hidden));
    assert!(!has(&default, &ignored));
    let hidden_only = overview(&map, Value::object([("include_hidden", Value::Bool(true))]));
    assert!(has(&hidden_only, &hidden));
    assert!(!has(&hidden_only, &ignored));
    let ignored_only = overview(
        &map,
        Value::object([("include_ignored", Value::Bool(true))]),
    );
    assert!(!has(&ignored_only, &hidden));
    assert!(has(&ignored_only, &ignored));
    let patterned = overview(
        &map,
        Value::object([
            ("pattern", Value::string("visible*.rs")),
            ("include_hidden", Value::Bool(true)),
            ("include_ignored", Value::Bool(true)),
        ]),
    );
    assert!(has(&patterned, &visible));
    assert!(!has(&patterned, &hidden));
    assert!(!has(&patterned, &ignored));
    assert!(has(
        &overview(&map, Value::object([("path", Value::string(".hidden.rs"))])),
        &hidden
    ));
    assert!(has(
        &overview(&map, Value::object([("path", Value::string("ignored.rs"))])),
        &ignored
    ));
}

#[test]
fn remember_with_known_id_replaces_the_note_without_adding_another() {
    let project = Directory::new();
    let home = Directory::new();
    write(&project, "src/main.rs", "fn main() {}\n");
    let map = new_map(&project, &home);
    let id = remember(&map, "Old title", "Old summary", "src/main.rs", None);
    let updated = call(
        &map,
        Value::object([
            ("action", Value::string("remember")),
            ("id", Value::string(&id)),
            ("title", Value::string("New title")),
            ("summary", Value::string("New summary")),
            (
                "sources",
                Value::Array(vec![source("src/main.rs", Some(1))]),
            ),
        ]),
    );
    assert_eq!(updated.get("id").and_then(Value::as_str), Some(id.as_str()));
    assert_eq!(updated.get("saved"), Some(&Value::Bool(true)));
    let result = overview(&new_map(&project, &home), Value::object([]));
    assert_eq!(result.get("notes_count"), Some(&Value::number(1)));
    assert_eq!(
        note(&result, &id).get("title").and_then(Value::as_str),
        Some("New title")
    );
    assert_eq!(
        note(&result, &id).get("summary").and_then(Value::as_str),
        Some("New summary")
    );
}

#[test]
fn later_pages_do_not_fingerprint_notes_skipped_by_offset() {
    let project = Directory::new();
    let home = Directory::new();
    for path in ["a.dat", "b.dat"] {
        fs::File::create(project.path().join(path))
            .unwrap()
            .set_len(33 * 1024 * 1024)
            .unwrap();
    }
    let map = new_map(&project, &home);
    let first = remember(&map, "Budget note A", "Large source A", "a.dat", None);
    let second = remember(&map, "Budget note B", "Large source B", "b.dat", None);
    for (offset, id) in [(1, first), (2, second)] {
        let result = overview(
            &map,
            Value::object([
                ("query", Value::string("Budget note")),
                ("limit", Value::number(1)),
                ("offset", Value::number(offset)),
            ]),
        );
        assert_eq!(entries(&result).len(), 1);
        assert_eq!(
            note(&result, &id).get("freshness").and_then(Value::as_str),
            Some("current")
        );
    }
}
