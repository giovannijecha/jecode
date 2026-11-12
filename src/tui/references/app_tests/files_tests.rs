//! The `@` browser over an isolated workspace.

use super::*;
use crate::test_support::link_directory;

/// A small workspace: folders, duplicate names, a space and accents, an empty folder,
/// and hidden and ignored entries that listings skip.
pub(super) fn workspace() -> Directory {
    let project = Directory::new();
    for (path, content) in [
        (".gitignore", "target/\n"),
        (".hidden/secret.txt", "secret"),
        ("target/out.txt", "build output"),
        ("docs/my file.md", "spaces"),
        ("docs/données.md", "accents"),
        ("src/main.rs", "fn main() {}"),
        ("src/tui/input.rs", "// input"),
        ("tools/main.rs", "fn main() {}"),
        ("README.md", "# Fixture"),
    ] {
        let path = project.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    fs::create_dir_all(project.path().join("empty")).unwrap();
    project
}

pub(super) fn open(app: &mut App) {
    typed(app, "@");
    assert!(files_open(app));
    settle(app);
}

fn files_open(app: &App) -> bool {
    app.state
        .selector
        .as_ref()
        .is_some_and(|menu| matches!(menu.purpose, Purpose::Files(_)))
}

fn browser(app: &mut App) -> &mut Browser {
    match &mut app.state.selector.as_mut().unwrap().purpose {
        Purpose::Files(browser) => browser,
        _ => panic!("the file selector"),
    }
}

/// Moves the focus to the row named `name`.
pub(super) fn focus(app: &mut App, name: &str) {
    let position = listed(app)
        .iter()
        .position(|row| row == name)
        .unwrap_or_else(|| panic!("{name} in {:?}", listed(app)));
    let menu = app.state.selector.as_mut().unwrap();
    menu.selected = 0;
    for _ in 0..position {
        key(app, DOWN);
    }
    assert_eq!(focused(app), name);
}

fn warning(app: &App) -> String {
    app.state
        .notice
        .as_ref()
        .map(|notice| notice.text.clone())
        .unwrap_or_default()
}

#[test]
fn the_root_lists_folders_first_without_hidden_or_ignored_entries() {
    let (home, project) = (Directory::new(), workspace());
    let mut app = app(&home, &project);
    typed(&mut app, "see ");
    open(&mut app);
    assert_eq!(
        listed(&app),
        ["docs/", "empty/", "src/", "tools/", "README.md"]
    );
    let rows = live(&app);
    assert!(
        rows.iter().any(|row| row.contains("─ Files · ")),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row.contains("5 items")), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains("  Filter")));
    assert!(rows.iter().any(|row| row.contains("Enter open")));
    assert!(rows.iter().any(|row| row.contains("draft kept")));
    assert_eq!(app.state.editor.text, "see @");
    focus(&mut app, "README.md");
    assert!(live(&app).iter().any(|row| row.contains("Enter insert")));
    nothing_sent(&app);
}

#[test]
fn folders_open_in_place_and_backspace_returns_with_focus_and_filter() {
    let (home, project) = (Directory::new(), workspace());
    let mut app = app(&home, &project);
    open(&mut app);
    focus(&mut app, "src/");
    key(&mut app, ENTER);
    assert!(files_open(&app), "a folder keeps the selector");
    settle(&mut app);
    assert_eq!(listed(&app), ["tui/", "main.rs"]);
    let rows = live(&app);
    assert!(rows.iter().any(|row| row.contains("/src")), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains("Backspace parent")));
    assert_eq!(app.state.editor.text, "@");

    typed(&mut app, "in");
    assert_eq!(listed(&app), ["tui/input.rs", "main.rs"]);
    key(&mut app, BACKSPACE);
    key(&mut app, BACKSPACE);
    assert_eq!(listed(&app), ["tui/", "main.rs"]);
    key(&mut app, BACKSPACE);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        "src/",
        "the folder just left keeps the focus"
    );
    key(&mut app, BACKSPACE);
    assert!(files_open(&app), "the root stops Backspace");

    typed(&mut app, "doc");
    assert_eq!(focused(&app), "docs/");
    key(&mut app, TAB);
    settle(&mut app);
    assert_eq!(listed(&app), ["données.md", "my file.md"]);
    key(&mut app, BACKSPACE);
    settle(&mut app);
    let menu = app.state.selector.as_ref().unwrap();
    assert_eq!(menu.editor.text, "doc", "returning restores the filter");
    assert_eq!(focused(&app), "docs/");
    key(&mut app, ESC);
    assert_eq!(app.state.editor.text, "@");
    nothing_sent(&app);
}

#[test]
fn a_filter_searches_the_subtree_and_shows_folders_for_duplicate_names() {
    let (home, project) = (Directory::new(), workspace());
    let mut app = app(&home, &project);
    open(&mut app);
    typed(&mut app, "main");
    assert!(!app.references_pending(), "filtering never starts a walk");
    assert_eq!(listed(&app), ["src/main.rs", "tools/main.rs"]);
    let rows = live(&app);
    assert!(
        rows.iter().any(|row| row.contains("2 matches")),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row.contains("› src/main.rs")));
    assert!(rows.iter().any(|row| row.contains("  tools/main.rs")));
    focus(&mut app, "tools/main.rs");
    key(&mut app, ENTER);
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor.text, "@tools/main.rs ");

    // Within a folder the rows are relative to it; the reference is not.
    app.state.editor.take();
    open(&mut app);
    focus(&mut app, "src/");
    key(&mut app, ENTER);
    settle(&mut app);
    typed(&mut app, "input");
    assert_eq!(listed(&app), ["tui/input.rs"]);
    key(&mut app, ENTER);
    assert_eq!(app.state.editor.text, "@src/tui/input.rs ");
    nothing_sent(&app);
}

#[test]
fn a_capped_subtree_keeps_matching_direct_children() {
    let (home, project) = (Directory::new(), workspace());
    let mut app = app(&home, &project);
    open(&mut app);
    typed(&mut app, "README");
    assert_eq!(listed(&app), ["README.md"]);

    // A depth-first walk can exhaust its cap inside an earlier sibling.
    let state = browser(&mut app);
    state.subtree = vec![Entry {
        path: "docs/deep.rs".into(),
        kind: Kind::File,
    }];
    state.capped = true;
    app.state.selector.as_mut().unwrap().refresh_files(true);
    assert_eq!(listed(&app), ["README.md"]);

    browser(&mut app).subtree.push(Entry {
        path: "README.md".into(),
        kind: Kind::File,
    });
    app.state.selector.as_mut().unwrap().refresh_files(true);
    assert_eq!(
        listed(&app),
        ["README.md"],
        "a delivered child is not repeated"
    );
}

#[test]
fn typed_paths_spaces_accents_and_separators_insert_unambiguous_references() {
    let (home, project) = (Directory::new(), workspace());
    let mut app = app(&home, &project);
    for (filter, reference) in [
        (r"docs\my file", r#"@"docs/my file.md" "#),
        ("donn", "@docs/données.md "),
        ("./src/tui/input.rs", "@src/tui/input.rs "),
        (r"src\tui\input.rs", "@src/tui/input.rs "),
    ] {
        app.state.editor.take();
        open(&mut app);
        typed(&mut app, filter);
        key(&mut app, ENTER);
        assert_eq!(app.state.editor.text, reference, "{filter}");
    }
    nothing_sent(&app);
}

#[test]
fn explicit_paths_reach_hidden_and_ignored_entries_inside_the_workspace_only() {
    let (home, project) = (Directory::new(), workspace());
    let outside = Directory::new();
    fs::write(outside.path().join("far.txt"), "far").unwrap();
    let mut app = app(&home, &project);
    open(&mut app);
    typed(&mut app, "target");
    assert_eq!(listed(&app)[0], "target/");
    key(&mut app, ENTER);
    settle(&mut app);
    assert_eq!(listed(&app), ["out.txt"]);
    key(&mut app, ENTER);
    assert_eq!(app.state.editor.text, "@target/out.txt ");

    app.state.editor.take();
    open(&mut app);
    typed(&mut app, ".hidden/secret.txt");
    assert_eq!(listed(&app), [".hidden/secret.txt"]);
    key(&mut app, ENTER);
    assert_eq!(app.state.editor.text, "@.hidden/secret.txt ");

    app.state.editor.take();
    open(&mut app);
    typed(&mut app, &outside.path().join("far.txt").to_string_lossy());
    assert!(listed(&app).is_empty());
    key(&mut app, ENTER);
    assert!(files_open(&app), "nothing outside the workspace is offered");
    assert_eq!(app.state.editor.text, "@");
}

#[test]
fn tab_retargets_a_quoted_file_reference_without_changing_surrounding_text() {
    let (home, project) = (Directory::new(), workspace());
    let mut app = app(&home, &project);
    let draft = r#"Read @"docs/my file.md", then continue"#;
    app.state.editor.insert(draft);
    app.state.editor.cursor = 10;
    app.edited();
    key(&mut app, TAB);
    settle(&mut app);
    assert_eq!(listed(&app), ["docs/my file.md"]);
    key(&mut app, ENTER);
    assert_eq!(app.state.editor.text, draft);
    assert_eq!(
        &app.state.editor.text[app.state.editor.cursor..],
        ", then continue"
    );
    nothing_sent(&app);
}

#[cfg(unix)]
#[test]
fn quoted_unix_filename_characters_survive_selection_and_retargeting() {
    let (home, project) = (Directory::new(), workspace());
    for name in [r#""quoted".txt"#, r"back\slash.txt"] {
        fs::write(project.path().join(name), "fixture").unwrap();
        let mut app = app(&home, &project);
        open(&mut app);
        let reference = syntax::file(name).unwrap();
        typed(&mut app, &reference);
        assert_eq!(listed(&app), [name]);
        key(&mut app, ENTER);
        assert_eq!(app.state.editor.text, format!("{reference} "));
        app.state.editor.cursor = 2;
        key(&mut app, TAB);
        settle(&mut app);
        assert_eq!(listed(&app), [name]);
        key(&mut app, ENTER);
        assert_eq!(app.state.editor.text, format!("{reference} "));
    }
}

#[test]
fn links_are_listed_as_entries_without_traversal() {
    let (home, project) = (Directory::new(), workspace());
    let outside = Directory::new();
    fs::write(outside.path().join("far.txt"), "far").unwrap();
    link_directory(&project.path().join("src"), &project.path().join("inner"));
    link_directory(outside.path(), &project.path().join("outer"));
    let mut app = app(&home, &project);
    open(&mut app);
    let rows = listed(&app);
    assert!(rows.contains(&"inner".to_string()) && rows.contains(&"outer".to_string()));
    assert!(
        live(&app)
            .iter()
            .any(|row| row.contains("outer") && row.trim_end().ends_with("link"))
    );
    typed(&mut app, "far");
    assert!(listed(&app).is_empty(), "a link is never walked");
    key(&mut app, BACKSPACE);
    key(&mut app, BACKSPACE);
    key(&mut app, BACKSPACE);
    focus(&mut app, "outer");
    key(&mut app, ENTER);
    // The reference names the link; reading it stays the file tools' decision.
    assert_eq!(app.state.editor.text, "@outer ");
}

#[test]
fn typed_link_names_keep_the_alias_without_browsing_through_it() {
    let (home, project) = (Directory::new(), workspace());
    link_directory(&project.path().join("src"), &project.path().join("alias"));
    let mut app = app(&home, &project);
    open(&mut app);
    typed(&mut app, "alias/main.rs");
    assert!(
        listed(&app).is_empty(),
        "an explicit path cannot descend through a link"
    );

    for _ in 0.."alias/main.rs".len() {
        key(&mut app, BACKSPACE);
    }
    typed(&mut app, "alias/");
    assert_eq!(listed(&app), ["alias"]);
    key(&mut app, ENTER);
    assert!(
        app.state.selector.is_none(),
        "a link does not open as a folder"
    );
    assert_eq!(app.state.editor.text, "@alias ");
}

#[test]
fn removed_entries_and_unreadable_folders_keep_the_draft() {
    let (home, project) = (Directory::new(), workspace());
    let mut app = app(&home, &project);
    typed(&mut app, "keep ");
    open(&mut app);
    fs::remove_file(project.path().join("README.md")).unwrap();
    focus(&mut app, "README.md");
    key(&mut app, ENTER);
    assert!(files_open(&app));
    assert!(warning(&app).contains("README.md no longer exists"));
    settle(&mut app);
    assert!(!listed(&app).contains(&"README.md".to_string()));
    assert_eq!(app.state.editor.text, "keep @");

    focus(&mut app, "empty/");
    key(&mut app, ENTER);
    settle(&mut app);
    assert!(live(&app).iter().any(|row| row.contains("Empty folder")));
    fs::remove_dir(project.path().join("empty")).unwrap();
    browser(&mut app).load();
    settle(&mut app);
    let rows = live(&app);
    assert!(
        rows.iter().any(|row| row.contains("Could not")),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row.contains("Backspace parent")));
    key(&mut app, ENTER);
    assert!(files_open(&app));
    key(&mut app, BACKSPACE);
    settle(&mut app);
    assert!(!listed(&app).contains(&"empty/".to_string()));
    key(&mut app, ESC);
    assert_eq!(app.state.editor.text, "keep @");
    nothing_sent(&app);
}

#[test]
fn a_newer_folder_or_another_conversation_discards_earlier_results() {
    let (home, project) = (Directory::new(), workspace());
    let mut app = app(&home, &project);
    let origin = Origin {
        draft: Editor::default(),
        start: 0,
        end: 0,
    };
    let mut quick = Browser::new(origin, app.archive.directory.clone(), None);
    quick.directory = "src".into();
    quick.load();
    let started = Instant::now();
    while quick.pending() {
        quick.poll();
        assert!(started.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(5));
    }
    let paths: Vec<_> = quick
        .children
        .iter()
        .flatten()
        .chain(&quick.subtree)
        .map(|entry| entry.path.clone())
        .collect();
    assert!(!paths.is_empty());
    assert!(
        paths.iter().all(|path| path.starts_with("src/")),
        "{paths:?}"
    );

    typed(&mut app, "x ");
    typed(&mut app, "@");
    browser(&mut app).session = Some("another-conversation".into());
    app.poll().unwrap();
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor.text, "x @");
}
