use super::*;
use crate::test_support::Directory;

fn install(home: &Path, name: &str, description: &str, body: &str) -> PathBuf {
    let package = home.join("skills").join(name);
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}"),
    )
    .unwrap();
    package
}

#[test]
fn empty_and_malformed_siblings_keep_a_sorted_useful_catalog() {
    let home = Directory::new();
    let mut library = Library::default();
    library.configure(home.path());
    assert!(!library.refresh());
    assert!(library.list().is_empty());
    install(home.path(), "zeta", "Last skill", "# Last");
    install(home.path(), "alpha", "First skill", "# First");
    fs::create_dir_all(home.path().join("skills/broken")).unwrap();
    fs::write(home.path().join("skills/broken/SKILL.md"), "No frontmatter").unwrap();
    assert!(library.refresh());
    assert_eq!(
        library
            .list()
            .iter()
            .map(|info| info.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
    assert_eq!(library.warnings().len(), 1);
    assert!(library.warnings()[0].contains("broken"));
    assert!(library.instructions().contains("skill:alpha"));
    assert!(!library.instructions().contains("No frontmatter"));
}

#[test]
fn reads_are_paged_and_stable_until_refresh() {
    let home = Directory::new();
    let package = install(home.path(), "review", "Review code", "# Review");
    fs::create_dir(package.join("references")).unwrap();
    fs::write(package.join("references/details.md"), "first\nsecond\n").unwrap();
    let mut library = Library::default();
    library.configure(home.path());
    library.refresh();
    let args = Value::object([("limit", Value::number(1))]);
    let first = library
        .read(
            "skill:review/references/details.md",
            &args,
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        first.get("content").and_then(Value::as_str),
        Some("1: first\n")
    );
    assert_eq!(first.get("next_offset").and_then(Value::as_usize), Some(2));
    fs::write(package.join("references/details.md"), "changed\n").unwrap();
    let second = library
        .read(
            "skill:review/references/details.md",
            &Value::object([("offset", Value::number(2))]),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        second.get("content").and_then(Value::as_str),
        Some("2: second\n")
    );
    library.refresh();
    let changed = library
        .read(
            "skill:review/references/details.md",
            &args,
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        changed.get("content").and_then(Value::as_str),
        Some("1: changed\n")
    );
}

#[test]
fn invalid_resource_paths_cannot_escape_or_open_config() {
    let home = Directory::new();
    let package = install(home.path(), "safe", "Safe skill", "# Safe");
    fs::write(home.path().join("config.json"), "secret fixture").unwrap();
    let mut library = Library::default();
    library.configure(home.path());
    library.refresh();
    let args = Value::object([]);
    for reference in [
        "skill:safe/../config.json",
        "skill:safe/../../config.json",
        "skill:safe/C:/config.json",
        "skill:safe/",
        "skill:../config.json",
    ] {
        assert!(
            library
                .read(reference, &args, &Cancellation::default())
                .is_err()
        );
    }
    let main = library
        .read("skill:safe", &args, &Cancellation::default())
        .unwrap();
    assert!(
        main.get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("# Safe")
    );
    assert_eq!(
        main.get("package_path").and_then(Value::as_str),
        Some(external_path(&fs::canonicalize(package).unwrap()).as_str())
    );
}

#[test]
fn frontmatter_rejects_control_characters_and_multiline_markers() {
    assert!(parse_metadata("---\nname: demo\ndescription: \u{1b}[31m\n---\n", "demo").is_err());
    assert!(parse_metadata("---\nname: demo\ndescription: >\n---\n", "demo").is_err());
    assert!(parse_metadata("---\nname: demo\ndescription: \"unfinished\n---\n", "demo").is_err());
    assert_eq!(
        parse_metadata(
            "---\nname: demo\ndescription: \"Review the user's 'change'\"\n---\n",
            "demo"
        )
        .unwrap()
        .description,
        "Review the user's 'change'"
    );
}

#[cfg(unix)]
#[test]
fn linked_library_is_usable_but_linked_resource_escape_is_rejected() {
    use std::os::unix::fs::symlink;
    let home = Directory::new();
    let external = Directory::new();
    let package = install(external.path(), "linked", "Linked skill", "# Linked");
    fs::write(home.path().join("config.json"), "secret fixture").unwrap();
    symlink(external.path().join("skills"), home.path().join("skills")).unwrap();
    symlink(home.path(), package.join("escape")).unwrap();
    let mut library = Library::default();
    library.configure(home.path());
    library.refresh();
    assert_eq!(library.list().len(), 1);
    assert!(
        library
            .read(
                "skill:linked/escape/config.json",
                &Value::object([]),
                &Cancellation::default()
            )
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn linked_package_and_header_cannot_expose_config_home() {
    use std::os::unix::fs::symlink;
    let home = Directory::new();
    let skills = home.path().join("skills");
    fs::create_dir(&skills).unwrap();
    fs::write(
        home.path().join("SKILL.md"),
        "---\nname: evil\ndescription: Evil.\n---\n",
    )
    .unwrap();
    symlink(home.path(), skills.join("evil")).unwrap();
    let package = skills.join("file_link");
    fs::create_dir(&package).unwrap();
    symlink(home.path().join("SKILL.md"), package.join("SKILL.md")).unwrap();
    let mut library = Library::default();
    library.configure(home.path());
    library.refresh();
    assert!(library.list().is_empty());
    assert_eq!(library.warnings().len(), 2);
}
