use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::test_support::Directory;
use crate::tools::Tools;

fn assert_dependency_rules_are_excluded(rules: &[u8]) {
    let directory = Directory::new();
    let root = directory.path();
    for (path, text) in [
        ("AGENTS.md", "Project rules"),
        ("src/AGENTS.md", "Source rules"),
        (".local/AGENTS.md", "Local rules"),
        ("target/AGENTS.md", "Build rules"),
    ] {
        let file = root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }
    fs::write(root.join(".gitignore"), ".local/\ntarget/\n").unwrap();
    let dependencies = [
        "node_modules/package",
        ".venv/lib/package",
        "vendor/package",
        "src/node_modules/package",
        "src/.venv/lib/package",
        "src/vendor/package",
    ];
    for path in dependencies {
        let folder = root.join(path);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("AGENTS.md"), rules).unwrap();
    }
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:9".into()),
        Tools::new(root).unwrap(),
    );
    agent.prepare_turn("Inspect the project").unwrap();
    let system = agent.current_system_message();
    let content = system
        .get("content")
        .and_then(crate::json::Value::as_str)
        .unwrap();
    for text in [
        "Project rules",
        "Source rules",
        "Local rules",
        "Build rules",
    ] {
        assert!(content.contains(text), "Missing {text}");
    }
    for path in dependencies {
        assert!(
            !content.contains(path),
            "Loaded dependency rules from {path}"
        );
    }
}

#[test]
fn dependency_instructions_do_not_enter_the_model_context() {
    assert_dependency_rules_are_excluded(b"Third-party package rules");
}

#[test]
fn invalid_dependency_instructions_do_not_block_turns() {
    assert_dependency_rules_are_excluded(&[0xff, 0xfe]);
}

#[test]
fn oversized_dependency_instructions_do_not_block_turns() {
    assert_dependency_rules_are_excluded(&vec![b'x'; BYTE_LIMIT + 1]);
}

#[test]
fn dependency_instructions_do_not_consume_the_combined_budget() {
    assert_dependency_rules_are_excluded(&vec![b'x'; BYTE_LIMIT / 2 + 1]);
}

#[test]
fn a_dependency_directory_can_be_the_explicit_launch_root() {
    let directory = Directory::new();
    let root = directory.path().join("vendor");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("AGENTS.md"), "Selected project rules").unwrap();
    let root = fs::canonicalize(root).unwrap();
    assert!(load(&root).unwrap().contains("Selected project rules"));
}
