use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::events::Event;
use crate::json::Value;
use crate::test_support::{Directory, HttpFixture, Request, completion, tool_call};
use crate::tools::Tools;

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
    assert!(!system(&requests[0]).contains("Project instructions from"));
    for request in &requests[1..3] {
        assert!(system(request).contains("Original project rules"));
        assert!(system(request).contains("Source: src/AGENTS.md"));
        assert!(system(request).contains("Directory scope: src and its descendants"));
        assert!(!system(request).contains("Revised project rules"));
        assert_eq!(
            system(request).matches("Project instructions from").count(),
            1
        );
    }
    assert!(system(&requests[3]).contains("Revised project rules"));
    assert!(!system(&requests[3]).contains("Original project rules"));
    assert!(!system(&requests[4]).contains("Project instructions from"));
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
fn resumed_sessions_reload_current_rules_without_rewriting_original_history() {
    let directory = Directory::new();
    let home = Directory::new();
    fs::create_dir(directory.path().join("src")).unwrap();
    let path = directory.path().join("src").join(FILE_NAME);
    fs::write(&path, "Earlier project rules").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("First answer", vec![])),
        (200, completion("Resumed answer", vec![])),
    ]);
    let mut first = agent(&directory, &fixture);
    first.enable_sessions(home.path()).unwrap();
    first.run_turn("Original request", &mut |_| Ok(())).unwrap();
    let original = first.messages.lock().unwrap().clone();
    let id = first.sessions().unwrap().id();
    drop(first);
    fs::write(&path, "Current project rules").unwrap();
    let mut resumed = agent(&directory, &fixture);
    resumed.enable_sessions(home.path()).unwrap();
    resumed.project_instructions = "Stale cached rules".into();
    resumed.resume(&id).unwrap();
    assert!(resumed.project_instructions.is_empty());
    assert_eq!(*resumed.messages.lock().unwrap(), original);
    resumed.run_turn("Continue", &mut |_| Ok(())).unwrap();
    assert_eq!(resumed.messages.lock().unwrap()[..original.len()], original);
    assert_eq!(
        resumed.sessions().unwrap().snapshot().messages[0],
        original[0]
    );
    let requests = fixture.finish();
    assert!(system(&requests[0]).contains("Earlier project rules"));
    assert!(system(&requests[1]).contains("Current project rules"));
    assert!(!system(&requests[1]).contains("Earlier project rules"));
    assert!(!system(&requests[1]).contains("Stale cached rules"));
    resumed.clear();
    assert!(resumed.project_instructions.is_empty());
    assert!(
        !resumed.messages.lock().unwrap()[0]
            .encode()
            .contains("Current project rules")
    );
}

#[test]
fn live_requests_keep_the_rules_after_context_compaction() {
    let directory = Directory::new();
    let home = Directory::new();
    fs::create_dir(directory.path().join("src")).unwrap();
    let instruction = directory.path().join("src").join(FILE_NAME);
    fs::write(&instruction, "Compaction project rules").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            400,
            Value::object([(
                "error",
                Value::object([
                    ("code", Value::string("context_length_exceeded")),
                    ("message", Value::string("Maximum context length exceeded")),
                ]),
            )]),
        ),
        (
            200,
            completion(&String::from("Continue inspection"), vec![]),
        ),
        (200, completion("Continue inspection", vec![])),
        (200, completion("Finished", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    agent.enable_sessions(home.path()).unwrap();
    agent.set_auto_compact(90);
    let temporary = agent.tools.temporary_instructions();
    let prompt = "original objective ".repeat(2500);
    let original = agent.messages.lock().unwrap().clone();
    let mut notices = Vec::new();
    let result = agent.run_turn(&prompt, &mut |event| {
        if let Event::Maintenance { ref text } = event {
            notices.push(text.clone());
            fs::write(&instruction, "Changed during compaction").unwrap();
        }
        if matches!(event, Event::Recovering { .. }) {
            return Err("Unexpected fixture request".into());
        }
        Ok(())
    });
    let requests = fixture.finish();
    assert!(
        result.is_ok(),
        "{result:?}; tools: {:?}; notices: {notices:?}",
        requests
            .iter()
            .map(|request| request.body.get("tools").is_some())
            .collect::<Vec<_>>()
    );
    assert!(agent.context.from > 1);
    assert!(!notices.is_empty());
    assert!(!agent.context.summary.is_empty());
    assert_eq!(agent.messages.lock().unwrap()[..original.len()], original);
    assert_eq!(requests.len(), 4);
    for request in [&requests[0], &requests[3]] {
        assert!(system(request).contains("Compaction project rules"));
        assert!(!system(request).contains("Changed during compaction"));
        assert!(system(request).contains("Source: src/AGENTS.md"));
        assert!(system(request).contains(&temporary));
        assert!(system(request).contains("useful durable tests"));
        assert!(system(request).contains("one-off verification"));
        assert_eq!(
            system(request)
                .matches("Working files and verification:")
                .count(),
            1
        );
        assert_eq!(
            system(request).matches("Project instructions from").count(),
            1
        );
    }
}

#[test]
fn new_sessions_do_not_save_the_previous_turns_rules_as_original_system_history() {
    let directory = Directory::new();
    let home = Directory::new();
    fs::write(directory.path().join(FILE_NAME), "Local project rules").unwrap();
    let fixture = HttpFixture::new(vec![(200, completion("Done", vec![]))]);
    let mut agent = agent(&directory, &fixture);
    agent.enable_sessions(home.path()).unwrap();
    let baseline = agent.messages.lock().unwrap()[0].clone();
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    assert!(!agent.project_instructions.is_empty());
    let previous = agent.sessions().unwrap().id();
    agent
        .start_new("fixture/model".into(), agent.effort())
        .unwrap();
    assert_ne!(agent.sessions().unwrap().id(), previous);
    assert!(agent.project_instructions.is_empty());
    assert_eq!(*agent.messages.lock().unwrap(), vec![baseline.clone()]);
    assert_eq!(
        agent.sessions().unwrap().snapshot().messages,
        vec![baseline.clone()]
    );
    agent.prepare_turn("New request").unwrap();
    assert_eq!(agent.sessions().unwrap().snapshot().messages[0], baseline);
    assert!(
        agent.transport_messages()[0]
            .encode()
            .contains("Local project rules")
    );
    assert_eq!(fixture.finish().len(), 1);
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
    assert!(system(&requests[0]).contains("Do not print [redacted]."));
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
    assert!(system(&requests[0]).contains("Unlocked project rules"));
}
