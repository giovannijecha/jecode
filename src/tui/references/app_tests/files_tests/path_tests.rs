use super::*;

#[test]
fn a_link_back_to_the_workspace_root_cannot_be_traversed() {
    let (home, project) = (Directory::new(), workspace());
    link_directory(project.path(), &project.path().join("loop"));
    let mut app = app(&home, &project);
    open(&mut app);
    typed(&mut app, "loop/src/main.rs");
    assert!(
        listed(&app).is_empty(),
        "a linked folder inside the workspace stays closed"
    );
    key(&mut app, ENTER);
    assert!(files_open(&app));
    assert_eq!(app.state.editor.text, "@");
    nothing_sent(&app);
}

#[cfg(windows)]
#[test]
fn a_typed_link_uses_its_stored_case_without_duplicate_rows() {
    let (home, project) = (Directory::new(), workspace());
    link_directory(&project.path().join("src"), &project.path().join("alias"));
    let mut app = app(&home, &project);
    open(&mut app);
    typed(&mut app, "ALIAS");
    assert_eq!(listed(&app), ["alias"]);
    key(&mut app, ENTER);
    assert_eq!(app.state.editor.text, "@alias ");
    assert!(
        app.state.selector.is_none(),
        "a link is inserted instead of opened"
    );
    nothing_sent(&app);
}

#[test]
fn absolute_paths_reach_the_workspace_directly_or_through_an_alias() {
    // macOS reaches temporary folders through `/tmp`: a linked prefix of the
    // workspace lies outside it and browses through no linked folder inside it.
    let (home, project) = (Directory::new(), workspace());
    let aliases = Directory::new();
    let alias = aliases.path().join("workspace");
    link_directory(project.path(), &alias);
    let mut app = app(&home, &project);
    for root in [project.path(), alias.as_path()] {
        app.state.editor.take();
        open(&mut app);
        typed(
            &mut app,
            &root.join("src").join("main.rs").to_string_lossy(),
        );
        assert_eq!(listed(&app), ["src/main.rs"], "through {root:?}");
        key(&mut app, ENTER);
        assert_eq!(app.state.editor.text, "@src/main.rs ");
    }
}

/// A case-insensitive file system accepts a typed path in any case; the reference
/// keeps the stored names, so it matches the listing and reads on every system.
#[cfg(windows)]
#[test]
fn explicit_paths_take_the_stored_case_of_their_names() {
    let (home, project) = (Directory::new(), workspace());
    let mut app = app(&home, &project);
    open(&mut app);
    typed(&mut app, "SRC/MAIN.RS");
    assert_eq!(listed(&app), ["src/main.rs"]);
    key(&mut app, ENTER);
    assert_eq!(app.state.editor.text, "@src/main.rs ");
}

#[cfg(windows)]
#[test]
fn a_typed_link_uses_its_unicode_stored_case() {
    let (home, project) = (Directory::new(), workspace());
    let stored = "caf\u{e9}";
    link_directory(&project.path().join("src"), &project.path().join(stored));
    let mut app = app(&home, &project);
    open(&mut app);
    typed(&mut app, "CAF\u{c9}");
    assert_eq!(listed(&app), [stored]);
    key(&mut app, ENTER);
    assert_eq!(app.state.editor.text, format!("@{stored} "));
}
