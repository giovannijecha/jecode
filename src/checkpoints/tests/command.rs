use super::*;
use crate::checkpoints::command::{offline, split};

fn words(list: &[&str]) -> Vec<String> {
    paths(list)
}

#[test]
fn words_split_on_spaces_outside_quotes() {
    assert_eq!(
        split(r#"restore cp-4 "src/a b.rs" --expected cp-9"#).unwrap(),
        ["restore", "cp-4", "src/a b.rs", "--expected", "cp-9"]
    );
    assert_eq!(
        split(r#"milestone "say \"hi\"""#).unwrap(),
        ["milestone", r#"say "hi""#]
    );
    assert_eq!(split(r#"milestone """#).unwrap(), ["milestone", ""]);
    assert!(split(r#"milestone "open"#).is_err());
    assert!(split("").unwrap().is_empty());
}

#[test]
fn exact_directory_restore_keeps_descendants_unchanged() {
    let fixture = Fixture::new();
    fixture.write("directory/file.txt", b"before\n");
    let workspace = fixture.open();
    let target = index(&workspace).latest();
    fixture.write("directory/file.txt", b"keep this edit\n");
    drop(workspace);
    let reference = crate::checkpoints::index::reference(target);
    let result = offline(
        fixture.home.path(),
        fixture.root.path(),
        &words(&["restore", &reference, "directory", "--exact-paths"]),
    )
    .unwrap();
    assert!(
        result.text().contains("restored 0 paths"),
        "{}",
        result.text()
    );
    assert_eq!(
        fixture.read("directory/file.txt").unwrap(),
        b"keep this edit\n"
    );
    offline(
        fixture.home.path(),
        fixture.root.path(),
        &words(&["restore", &reference, "directory"]),
    )
    .unwrap();
    assert_eq!(fixture.read("directory/file.txt").unwrap(), b"before\n");
}

#[test]
fn the_offline_command_ignores_provider_configuration() {
    let fixture = Fixture::new();
    fixture.write("a.txt", b"one\n");
    // A malformed provider configuration beside the archive is never read.
    fs::write(fixture.home.path().join("config.json"), "{broken").unwrap();
    let help = offline(fixture.home.path(), fixture.root.path(), &words(&["help"])).unwrap();
    assert!(help.text().contains("restore"), "{}", help.text());
    assert!(
        !fixture.home.path().join("checkpoints").exists(),
        "help opens no archive"
    );

    let status = offline(fixture.home.path(), fixture.root.path(), &[])
        .unwrap()
        .text();
    assert!(status.contains("off (offline command)"), "{status}");
    let milestone = offline(
        fixture.home.path(),
        fixture.root.path(),
        &words(&["milestone", "before cleanup"]),
    )
    .unwrap()
    .text();
    let reference = milestone
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .find(|word| word.starts_with("cp-"))
        .unwrap_or_else(|| panic!("{milestone}"))
        .to_string();

    fixture.write("a.txt", b"two\n");
    let listed = offline(
        fixture.home.path(),
        fixture.root.path(),
        &words(&["list", "a.txt"]),
    )
    .unwrap()
    .text();
    assert!(listed.contains("checkpoint:"), "{listed}");
    let shown = offline(
        fixture.home.path(),
        fixture.root.path(),
        &words(&["show", &reference, "a.txt"]),
    )
    .unwrap();
    assert!(
        shown.raw.as_deref().is_some_and(|raw| raw.contains("one")),
        "{:?}",
        shown.raw
    );

    let preview = offline(
        fixture.home.path(),
        fixture.root.path(),
        &words(&["restore", &reference, "a.txt", "--preview"]),
    )
    .unwrap();
    assert!(preview.text().contains("preview"), "{}", preview.text());
    assert_eq!(fixture.read("a.txt").unwrap(), b"two\n");
    offline(
        fixture.home.path(),
        fixture.root.path(),
        &words(&["restore", &reference, "a.txt"]),
    )
    .unwrap();
    assert_eq!(fixture.read("a.txt").unwrap(), b"one\n");
    assert_eq!(
        fs::read_to_string(fixture.home.path().join("config.json")).unwrap(),
        "{broken"
    );

    let unknown = offline(
        fixture.home.path(),
        fixture.root.path(),
        &words(&["explode"]),
    )
    .err()
    .unwrap();
    assert!(unknown.contains("explode"), "{unknown}");
    let option = offline(
        fixture.home.path(),
        fixture.root.path(),
        &words(&["list", "--bogus"]),
    )
    .err()
    .unwrap();
    assert!(option.contains("--bogus"), "{option}");
}

#[test]
fn empty_optional_restore_fields_preserve_default_conflict_checking() {
    use crate::checkpoints::actions::{self, Caller};
    use crate::json::Value;

    let fixture = Fixture::new();
    fixture.write("a.txt", b"original\n");
    let workspace = fixture.open();
    let target = workspace
        .mark("milestone", "before", "other", false, false)
        .unwrap();
    fixture.write("a.txt", b"colleague\n");
    workspace.verify().unwrap();
    let caller = Caller {
        origin: "model",
        user: false,
    };
    for action in ["restore", "restore_all"] {
        let selected = if action == "restore" {
            vec![Value::string("a.txt")]
        } else {
            vec![]
        };
        let result = actions::run(
            &workspace,
            &caller,
            &Value::object([
                ("action", Value::string(action)),
                ("ref", Value::string(format!("cp-{target}"))),
                ("paths", Value::Array(selected)),
                ("expected", Value::string("")),
            ]),
        )
        .unwrap();
        assert_eq!(
            result
                .get("conflicts")
                .and_then(Value::as_array)
                .map(<[_]>::len),
            Some(1),
            "{result:?}"
        );
        assert_eq!(fixture.read("a.txt").unwrap(), b"colleague\n");
    }
}

#[test]
fn empty_path_filters_query_the_scope_but_never_make_restore_or_capture_global() {
    use crate::checkpoints::actions::{self, Caller};
    use crate::json::Value;

    let fixture = Fixture::new();
    fixture.write("a.txt", b"original\n");
    let workspace = fixture.open();
    let target = workspace
        .mark("milestone", "before", "other", false, false)
        .unwrap();
    fixture.write("a.txt", b"colleague\n");
    let caller = Caller {
        origin: "model",
        user: false,
    };
    for action in ["list", "inspect", "restore", "capture"] {
        let result = actions::run(
            &workspace,
            &caller,
            &Value::object([
                ("action", Value::string(action)),
                ("ref", Value::string(format!("cp-{target}"))),
                ("paths", Value::Array(vec![])),
                ("expected", Value::string("latest")),
            ]),
        );
        assert_eq!(
            result.is_ok(),
            matches!(action, "list" | "inspect"),
            "{action}: {result:?}"
        );
        assert_eq!(fixture.read("a.txt").unwrap(), b"colleague\n");
    }
}
