use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::events::Event;
use crate::json::Value;
use crate::test_support::{Directory, HttpFixture, Request, completion, tool_call};
use crate::tools::Tools;

mod lifecycle;

fn agent(directory: &Directory, fixture: &HttpFixture) -> Agent {
    Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    )
}

fn system(request: &Request) -> &str {
    request.body.get("messages").unwrap().as_array().unwrap()[0]
        .get("content")
        .unwrap()
        .as_str()
        .unwrap()
}

/// The instructions in effect for a request: its latest snapshot.
fn current(request: &Request) -> String {
    crate::test_support::latest_snapshot(&request.body).unwrap_or_default()
}

#[test]
fn only_files_inside_launch_root_are_loaded_and_utf8_bom_is_accepted() {
    let directory = Directory::new();
    let project = directory.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(directory.path().join(FILE_NAME), "Parent rules").unwrap();
    fs::write(project.join("src").join(FILE_NAME), "Child rules").unwrap();
    fs::write(
        project.join("src").join("JECODE.md"),
        "Ignored legacy rules",
    )
    .unwrap();
    for name in ["RULES.md", "INSTRUCTIONS.md"] {
        fs::write(project.join(name), "Other project rules").unwrap();
    }
    let root = fs::canonicalize(project).unwrap();
    let loaded = load(&root).unwrap();
    assert!(loaded.contains("Source: src/AGENTS.md\nDirectory scope: src and its descendants"));
    assert!(loaded.contains("Child rules"));
    assert!(!loaded.contains("Parent rules"));
    assert!(!loaded.contains("Ignored legacy rules"));
    let path = root.join(FILE_NAME);
    fs::write(&path, "\u{feff}# Project\r\nKeep café names.\r\n").unwrap();
    let loaded = load(&root).unwrap();
    assert!(loaded.starts_with("Source: AGENTS.md\nDirectory scope: the entire project tree\n\n# Project\r\nKeep café names.\r\n"));
    assert!(loaded.contains("Child rules"));
    for empty in ["", "\u{feff} \r\n\t"] {
        fs::write(&path, empty).unwrap();
        assert!(load(&root).unwrap().contains("Child rules"));
    }
    fs::write(path, "x".repeat(BYTE_LIMIT)).unwrap();
    assert!(load(&root).unwrap_err().contains("combined 64 KiB"));
}

#[test]
fn nested_sections_are_parent_first_and_keep_literal_references() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    for path in ["z", "a/deep", ".git/hooks", "target"] {
        fs::create_dir_all(root.join(path)).unwrap();
    }
    fs::write(root.join(FILE_NAME), "Root @missing.md").unwrap();
    fs::write(root.join("a/AGENTS.md"), "A instructions").unwrap();
    fs::write(root.join("a/deep/AGENTS.md"), "Deep instructions").unwrap();
    fs::write(root.join("z/AGENTS.md"), "Z instructions").unwrap();
    fs::write(
        root.join("target/AGENTS.md"),
        "Ignored by Git, still loaded",
    )
    .unwrap();
    fs::write(root.join(".git/hooks/AGENTS.md"), "Git metadata rules").unwrap();
    fs::write(root.join("JECODE.md"), "Legacy rules").unwrap();
    let loaded = load(&root).unwrap();
    let positions = [
        "Source: AGENTS.md",
        "Source: a/AGENTS.md",
        "Source: a/deep/AGENTS.md",
        "Source: target/AGENTS.md",
        "Source: z/AGENTS.md",
    ]
    .map(|source| loaded.find(source).unwrap());
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(loaded.contains("Root @missing.md"));
    assert!(loaded.contains("Ignored by Git, still loaded"));
    assert!(!loaded.contains("Git metadata rules"));
    assert!(!loaded.contains("Legacy rules"));
}

#[test]
fn legacy_instruction_files_are_ignored_without_a_fallback() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("JECODE.md"), [0xff, 0xfe]).unwrap();
    fs::write(root.join("src/JECODE.md"), "Legacy child rules").unwrap();
    assert!(load(&root).unwrap().is_empty());

    fs::write(root.join(FILE_NAME), " \n").unwrap();
    assert!(load(&root).unwrap().is_empty());

    fs::write(root.join(FILE_NAME), "Current project rules").unwrap();
    let loaded = load(&root).unwrap();
    assert!(loaded.contains("Current project rules"));
    assert!(!loaded.contains("Legacy child rules"));
}

#[test]
fn a_directory_removed_during_discovery_does_not_block_other_instructions() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let missing = root.join("removed");
    let mut sections = Vec::new();
    let mut total = 0;
    scan(&root, &missing, &mut sections, &mut total).unwrap();
    assert!(sections.is_empty());
    assert_eq!(total, 0);
}

#[test]
fn nested_invalid_files_and_aggregate_overflow_fail_before_history_changes() {
    let directory = Directory::new();
    let child = directory.path().join("src");
    fs::create_dir(&child).unwrap();
    let root_file = directory.path().join(FILE_NAME);
    let child_file = child.join(FILE_NAME);
    fs::write(&root_file, "r".repeat(BYTE_LIMIT / 2 + 1)).unwrap();
    fs::write(&child_file, "c".repeat(BYTE_LIMIT / 2)).unwrap();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = agent(&directory, &fixture);
    let original = agent.messages.lock().unwrap().clone();
    let error = agent.prepare_turn("Inspect").unwrap_err();
    assert!(error.contains("combined 64 KiB"), "{error}");
    assert!(error.contains("src"), "{error}");
    assert_eq!(*agent.messages.lock().unwrap(), original);
    fs::remove_file(&root_file).unwrap();
    fs::write(&child_file, [0xff, 0xfe]).unwrap();
    let error = agent.prepare_turn("Inspect").unwrap_err();
    assert!(error.contains("src"), "{error}");
    assert!(error.contains("UTF-8"), "{error}");
    assert_eq!(*agent.messages.lock().unwrap(), original);
    assert!(fixture.finish().is_empty());
}

#[test]
fn invalid_files_stop_preparation_without_adding_history_or_contacting_the_model() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = agent(&directory, &fixture);
    let original = agent.messages.lock().unwrap().clone();
    let path = directory.path().join(FILE_NAME);
    fs::write(&path, [0xff, 0xfe]).unwrap();
    assert!(agent.prepare_turn("Inspect").unwrap_err().contains("UTF-8"));
    assert_eq!(*agent.messages.lock().unwrap(), original);
    fs::write(&path, vec![b'x'; BYTE_LIMIT + 1]).unwrap();
    assert!(
        agent
            .prepare_turn("Inspect")
            .unwrap_err()
            .contains("64 KiB")
    );
    assert_eq!(*agent.messages.lock().unwrap(), original);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(
        agent
            .prepare_turn("Inspect")
            .unwrap_err()
            .contains("regular UTF-8 text file")
    );
    assert_eq!(*agent.messages.lock().unwrap(), original);
    assert!(agent.prepared.is_none());
    assert!(fixture.finish().is_empty());
}

#[test]
fn each_turn_keeps_one_snapshot_and_the_next_turn_reloads_creation_edits_and_deletion() {
    let directory = Directory::new();
    fs::create_dir(directory.path().join("src")).unwrap();
    fs::write(directory.path().join("note.txt"), "A note").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("Before instructions", vec![])),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "read-note",
                    "read",
                    Value::object([("path", Value::string("note.txt"))]),
                )],
            ),
        ),
        (200, completion("Read the note", vec![])),
        (200, completion("After editing instructions", vec![])),
        (200, completion("After removing instructions", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    let original_system = agent.messages.lock().unwrap()[0].clone();
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    let path = directory.path().join("src").join(FILE_NAME);
    fs::write(&path, "Original project rules").unwrap();
    let mut edited = false;
    agent
        .run_turn("Read a note", &mut |event| {
            if matches!(event, Event::ToolFinished { .. }) {
                fs::write(&path, "Revised project rules").unwrap();
                edited = true;
            }
            Ok(())
        })
        .unwrap();
    assert!(edited);
    agent.run_turn("Third request", &mut |_| Ok(())).unwrap();
    fs::remove_file(path).unwrap();
    agent.run_turn("Fourth request", &mut |_| Ok(())).unwrap();
    assert_eq!(agent.messages.lock().unwrap()[0], original_system);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 5);
    assert!(current(&requests[0]).is_empty());
    for request in &requests[1..3] {
        assert!(current(request).contains("Original project rules"));
        assert!(current(request).contains("Source: src/AGENTS.md"));
        assert!(current(request).contains("Directory scope: src and its descendants"));
        assert!(!current(request).contains("Revised project rules"));
        // The tool-loop request reuses the turn's snapshot instead of adding one.
        assert_eq!(crate::test_support::snapshot_count(&request.body), 1);
    }
    assert!(current(&requests[3]).contains("Revised project rules"));
    assert!(!current(&requests[3]).contains("Original project rules"));
    assert!(current(&requests[4]).contains(super::super::environment::NO_INSTRUCTIONS));
    assert!(!current(&requests[4]).contains("project rules"));
    for request in &requests {
        assert!(!system(request).contains("project rules"));
    }
}

#[test]
fn changed_rules_invalidate_measurements_and_unchanged_rules_keep_them() {
    let directory = Directory::new();
    let path = directory.path().join(FILE_NAME);
    fs::write(&path, "Short project rules").unwrap();
    let measured = || {
        let mut reply = completion("Done", vec![]);
        if let Value::Object(fields) = &mut reply {
            fields.insert(
                "usage".into(),
                Value::object([
                    ("prompt_tokens", Value::number(1000)),
                    ("completion_tokens", Value::number(10)),
                ]),
            );
        }
        (200, reply)
    };
    let fixture = HttpFixture::new(vec![measured(), measured(), measured()]);
    let mut agent = agent(&directory, &fixture);
    agent.run_turn("First", &mut |_| Ok(())).unwrap();
    let before = agent.locked_estimate();
    let calibration = agent.context.calibration;
    assert!(calibration.is_some());
    agent.context.ceiling = Some(100_000);
    fs::write(&path, "Keep this project convention.\n".repeat(1500)).unwrap();
    agent.prepare_turn("Second").unwrap();
    assert_eq!(agent.context.input_tokens, None);
    assert_eq!(agent.context.measured_end, 0);
    assert_eq!(agent.context.calibration, calibration);
    assert_eq!(agent.context.ceiling, Some(100_000));
    // The new rules are 45,000 bytes; the estimate charges them at least at
    // the calibrated rate.
    assert!(agent.locked_estimate() > before + agent.context.estimate_bytes(45_000));
    agent.run_turn("Second", &mut |_| Ok(())).unwrap();
    let calibration = agent.context.calibration;
    let measured_end = agent.context.measured_end;
    agent.prepare_turn("Third").unwrap();
    assert_eq!(agent.context.input_tokens, Some(1000));
    assert_eq!(agent.context.measured_end, measured_end);
    assert_eq!(agent.context.calibration, calibration);
    agent.run_turn("Third", &mut |_| Ok(())).unwrap();
    fs::remove_file(path).unwrap();
    agent.prepare_turn("Without project rules").unwrap();
    assert_eq!(agent.context.input_tokens, None);
    assert_eq!(agent.context.measured_end, 0);
    assert!(agent.project_instructions.is_empty());
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn source_edits_invalidate_measurements_even_if_redacted_projection_is_identical() {
    let directory = Directory::new();
    let path = directory.path().join(FILE_NAME);
    fs::write(&path, "isolated-fixture-key").unwrap();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = agent(&directory, &fixture);
    agent.refresh_project_instructions().unwrap();
    let projected = agent.redact(&agent.project_instructions);
    agent.context.input_tokens = Some(1000);
    agent.context.measured_end = 1;
    fs::write(&path, "[redacted]").unwrap();
    agent.refresh_project_instructions().unwrap();
    assert_eq!(agent.redact(&agent.project_instructions), projected);
    assert_eq!(agent.context.input_tokens, None);
    assert_eq!(agent.context.measured_end, 0);
    assert!(fixture.finish().is_empty());
}

#[test]
fn configured_credentials_are_masked_in_project_instructions() {
    let directory = Directory::new();
    fs::write(
        directory.path().join(FILE_NAME),
        "Do not print isolated-fixture-key.",
    )
    .unwrap();
    let fixture = HttpFixture::new(vec![(200, completion("Done", vec![]))]);
    let mut agent = agent(&directory, &fixture);
    agent.run_turn("Inspect", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert!(current(&requests[0]).contains("Do not print [redacted]."));
    assert!(!requests[0].body.encode().contains("isolated-fixture-key"));
}

#[cfg(windows)]
#[test]
fn an_unreadable_file_stops_the_request_and_can_be_retried_after_unlocking() {
    use std::os::windows::fs::OpenOptionsExt;

    let directory = Directory::new();
    let path = directory.path().join(FILE_NAME);
    fs::write(&path, "Unlocked project rules").unwrap();
    let fixture = HttpFixture::new(vec![(200, completion("Retried", vec![]))]);
    let mut agent = agent(&directory, &fixture);
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap();
    assert!(agent.prepare_turn("Inspect").is_err());
    assert_eq!(agent.messages.lock().unwrap().len(), 1);
    assert!(agent.prepared.is_none());
    drop(lock);
    agent.run_turn("Inspect", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(current(&requests[0]).contains("Unlocked project rules"));
}
