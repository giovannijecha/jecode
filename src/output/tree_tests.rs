//! Resolution across one conversation tree keeps the ownership and link
//! checks of a store's own outputs.

use super::*;
use crate::{
    json::Value,
    test_support::{Directory, link_directory},
};
use std::fs;
use std::path::Path;

fn owned(root: &Path, project: &Path, session: &str) -> Store {
    Store::for_session(
        root.to_path_buf(),
        project.to_path_buf(),
        session,
        Redactor::empty(),
    )
    .unwrap()
}

fn saved(store: &Store, text: &[u8]) -> String {
    let mut logs = store.create().unwrap();
    logs.stdout.write(text).unwrap();
    logs.stdout.finish().unwrap();
    logs.stdout_ref.clone()
}

#[test]
fn collaborator_outputs_need_their_own_ownership_record() {
    let home = Directory::new();
    let project = Directory::new();
    let collaborators = home.path().join("child-home");
    let main = owned(&home.path().join("outputs"), project.path(), "1-1-1")
        .with_collaborators(collaborators.clone());
    let child_root = collaborators.join("sessions/folder/outputs");
    let reference = saved(&owned(&child_root, project.path(), "2-2-2"), b"child");
    assert_eq!(
        fs::read(main.resolve(&reference).unwrap()).unwrap(),
        b"child"
    );

    let marker = child_root.join("2-2-2/.jecode-output.json");
    let original = fs::read(&marker).unwrap();
    let foreign = Value::object([
        ("format", Value::string("jecode.output")),
        ("format_version", Value::number(1)),
        ("directory", Value::string(project.path().to_string_lossy())),
        ("session_id", Value::string("3-3-3")),
    ]);
    fs::write(&marker, foreign.encode()).unwrap();
    let error = main.resolve(&reference).unwrap_err();
    assert!(error.contains("another folder or session"), "{error}");
    fs::write(&marker, original).unwrap();
    assert!(main.resolve(&reference).is_ok());
}

#[test]
fn links_in_the_conversation_tree_are_never_followed() {
    let home = Directory::new();
    let project = Directory::new();
    let outside = Directory::new();
    let collaborators = home.path().join("child-home");
    let main = owned(&home.path().join("outputs"), project.path(), "1-1-1")
        .with_collaborators(collaborators.clone());
    fs::create_dir_all(collaborators.join("sessions/folder")).unwrap();

    // A linked collaborator folder is skipped without being entered.
    let linked = saved(
        &owned(&outside.path().join("a/outputs"), project.path(), "2-2-2"),
        b"outside",
    );
    link_directory(
        &outside.path().join("a"),
        &collaborators.join("sessions/linked"),
    );
    let error = main.resolve(&linked).unwrap_err();
    assert!(error.contains("not saved for this conversation"), "{error}");

    // A link deeper in a collaborator's path refuses the reference.
    let deeper = saved(
        &owned(&outside.path().join("b"), project.path(), "4-4-4"),
        b"outside",
    );
    link_directory(
        &outside.path().join("b"),
        &collaborators.join("sessions/folder/outputs"),
    );
    let error = main.resolve(&deeper).unwrap_err();
    assert!(error.contains("link or junction"), "{error}");

    // So does a linked root of the agent above a collaborator.
    let above = home.path().join("above");
    let above_reference = saved(
        &owned(&outside.path().join("c"), project.path(), "5-5-5"),
        b"outside",
    );
    link_directory(&outside.path().join("c"), &above);
    let mut child = owned(&home.path().join("child"), project.path(), "6-6-6");
    child.inherit(&owned(&above, project.path(), "5-5-5"));
    let error = child.resolve(&above_reference).unwrap_err();
    assert!(error.contains("link or junction"), "{error}");
}

#[test]
fn collaborators_reach_only_the_sessions_above_them() {
    let home = Directory::new();
    let project = Directory::new();
    let root = home.path().join("outputs");
    let main = owned(&root, project.path(), "1-1-1");
    let main_reference = saved(&main, b"main");
    let unrelated = saved(&owned(&root, project.path(), "7-7-7"), b"other");
    let mut child = owned(&home.path().join("child"), project.path(), "2-2-2");
    child.inherit(&main);
    assert_eq!(
        fs::read(child.resolve(&main_reference).unwrap()).unwrap(),
        b"main"
    );
    let error = child.resolve(&unrelated).unwrap_err();
    assert!(error.contains("not saved for this conversation"), "{error}");
    let invalid = child.resolve("output:1-1-1:x:stdout").unwrap_err();
    assert!(invalid.contains("exact output: value"), "{invalid}");
}
