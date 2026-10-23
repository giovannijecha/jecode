use super::*;
use crate::test_support::Directory;
use std::fs;

fn undo(path: &str) -> String {
    undo_with_prefix(path, "/checkpoints")
}

fn undo_with_prefix(path: &str, prefix: &str) -> String {
    let value = Value::object([(
        "undo",
        Value::object([
            ("action", Value::string("restore")),
            ("ref", Value::string("rc-test")),
            ("paths", Value::Array(vec![Value::string(path)])),
            ("expected", Value::string("fingerprint")),
        ]),
    )]);
    render::details(&value, prefix)
        .into_iter()
        .find(|(label, _)| label == "undo")
        .unwrap()
        .1
}

fn parsed_undo_path(command: &str) -> String {
    let words = split(command).unwrap();
    assert_eq!(words[0], "/checkpoints");
    let Parsed::Action(args) = parse(&words[1..]).unwrap() else {
        panic!("expected restore action")
    };
    assert_eq!(
        args.get("expected").and_then(Value::as_str),
        Some("fingerprint")
    );
    args.get("paths")
        .and_then(Value::as_array)
        .and_then(|paths| paths.first())
        .and_then(Value::as_str)
        .unwrap()
        .into()
}

#[test]
fn undo_round_trips_option_shaped_path() {
    let command = undo("--draft");
    assert!(command.contains("--expected fingerprint -- --draft"));
    assert_eq!(parsed_undo_path(&command), "--draft");
    let words = split("show rc-test -- --draft").unwrap();
    assert!(matches!(parse(&words), Ok(Parsed::Show { path, .. }) if path == "--draft"));
}

#[test]
fn undo_round_trips_quoted_trailing_backslash() {
    let path = "a b\\";
    assert_eq!(parsed_undo_path(&undo(path)), path);
}

#[test]
fn offline_commands_quote_shell_metacharacters() {
    let path = "x;$(touch sentinel)' y";
    let command = undo_with_prefix(path, "jecode checkpoints");
    #[cfg(windows)]
    assert_eq!(
        command,
        "jecode checkpoints restore rc-test --expected 'fingerprint' -- 'x;$(touch sentinel)'' y'"
    );
    #[cfg(not(windows))]
    assert_eq!(
        command,
        "jecode checkpoints restore rc-test --expected 'fingerprint' -- 'x;$(touch sentinel)'\\'' y'"
    );
    assert_eq!(render::quote("x;touch", "jecode checkpoints"), "'x;touch'");
}

#[test]
fn local_show_pages_and_offline_show_streams_full_text() {
    let fixture = Directory::new();
    let home = fixture.path().join("home");
    let workspace = fixture.path().join("workspace");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&workspace).unwrap();
    let content = (1..=220)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    fs::write(workspace.join("note.txt"), &content).unwrap();
    let store = Store::open(&home, &workspace).unwrap();
    let reference = store
        .capture(
            "write",
            "test",
            &["note.txt".into()],
            &Cancellation::default(),
        )
        .unwrap()
        .reference
        .unwrap();
    let words = vec!["show".into(), reference, "note.txt".into()];
    let first = run(&store, &words).unwrap();
    assert!(first.raw.as_deref().unwrap().contains("200: line 200"));
    assert!(!first.raw.as_deref().unwrap().contains("201: line 201"));
    let next = &first
        .details
        .iter()
        .find(|(label, _)| label == "next page")
        .unwrap()
        .1;
    assert!(next.contains("--offset 201"));
    let next_words = split(next).unwrap();
    let second = run(&store, &next_words[1..]).unwrap();
    assert!(second.raw.as_deref().unwrap().contains("201: line 201"));
    assert!(second.details.iter().all(|(label, _)| label != "next page"));

    let first_seven = run(
        &store,
        &[
            "show".into(),
            words[1].clone(),
            "note.txt".into(),
            "--limit".into(),
            "7".into(),
        ],
    )
    .unwrap();
    let next_seven = &first_seven
        .details
        .iter()
        .find(|(label, _)| label == "next page")
        .unwrap()
        .1;
    assert!(next_seven.contains("--offset 8 --limit 7"));
    let next_seven_words = split(next_seven).unwrap();
    assert!(
        run(&store, &next_seven_words[1..])
            .unwrap()
            .raw
            .as_deref()
            .unwrap()
            .contains("8: line 8")
    );
    let offline_page = offline(
        &home,
        &workspace,
        &[
            "show".into(),
            words[1].clone(),
            "note.txt".into(),
            "--limit".into(),
            "7".into(),
        ],
    )
    .unwrap();
    let offline_next = &offline_page
        .details
        .iter()
        .find(|(label, _)| label == "next page")
        .unwrap()
        .1;
    assert!(offline_next.starts_with("jecode checkpoints show "));
    assert!(offline_next.ends_with("-- 'note.txt'"));

    let mut full = Vec::new();
    assert!(stream_show(&home, &workspace, &words, &mut full).unwrap());
    assert_eq!(full, content.as_bytes());
    assert!(!stream_show(&home, &workspace, &next_words[1..], &mut Vec::new()).unwrap());
    let paged = offline(&home, &workspace, &next_words[1..]).unwrap();
    assert!(paged.raw.as_deref().unwrap().contains("201: line 201"));
}

#[test]
fn local_show_continues_a_long_line_by_byte_offset() {
    let fixture = Directory::new();
    let home = fixture.path().join("home");
    let workspace = fixture.path().join("workspace");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&workspace).unwrap();
    fs::write(
        workspace.join("long.txt"),
        format!("{}\n", "x".repeat(70_000)),
    )
    .unwrap();
    let store = Store::open(&home, &workspace).unwrap();
    let reference = store
        .capture(
            "write",
            "test",
            &["long.txt".into()],
            &Cancellation::default(),
        )
        .unwrap()
        .reference
        .unwrap();
    let first = run(&store, &["show".into(), reference, "long.txt".into()]).unwrap();
    let next = &first
        .details
        .iter()
        .find(|(label, _)| label == "next page")
        .unwrap()
        .1;
    assert!(next.contains("--byte-offset"));
    let words = split(next).unwrap();
    let second = run(&store, &words[1..]).unwrap();
    assert!(second.raw.as_deref().unwrap().contains('x'));
}
