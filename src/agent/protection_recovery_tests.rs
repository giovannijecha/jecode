use super::*;
use crate::test_support::{Directory, tool_call};
use std::fs;

fn interrupted_registration(copy: bool, partial: bool) -> (Directory, Directory, String, Value) {
    interrupted_paths(copy, partial, &["protected"])
}

fn interrupted_paths(
    copy: bool,
    partial: bool,
    paths: &[&str],
) -> (Directory, Directory, String, Value) {
    let directory = Directory::new();
    let home = Directory::new();
    for path in paths {
        fs::write(directory.path().join(path), b"original\r\nlast").unwrap();
    }
    let mut first = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    first.enable_sessions(home.path()).unwrap();
    first
        .prepare_turn("Keep protected exactly unchanged.")
        .unwrap();
    let arguments = Value::object([
        ("action", Value::string("record")),
        (
            "paths",
            Value::Array(paths.iter().map(|path| Value::string(*path)).collect()),
        ),
        ("reason", Value::string("User preservation constraint")),
    ]);
    first.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("")),
        (
            "tool_calls",
            Value::Array(vec![tool_call("register", "protect", arguments.clone())]),
        ),
    ]));
    first
        .checkpoint(crate::sessions::Stage::Tool("register".into()))
        .unwrap();
    if copy {
        let result = first.tools.execute("protect", &arguments.encode());
        assert!(result.get("error").is_none(), "{result:?}");
        if partial {
            let entry = &result.get("file_protections").unwrap().as_array().unwrap()[0];
            let id = entry.get("snapshot_id").and_then(Value::as_str).unwrap();
            let outputs = first.sessions().unwrap().store().output_directory();
            let saved = outputs
                .join(first.sessions().unwrap().id())
                .join(format!("{id}.baseline"));
            fs::write(saved, b"partial").unwrap();
        }
    }
    // Stop after the started-tool checkpoint without recording its result.
    let id = first.sessions().unwrap().id();
    drop(first);
    (directory, home, id, arguments)
}

#[test]
fn corrupted_intents_remain_unknown_even_when_the_remaining_json_is_structurally_valid() {
    for valid_json in [false, true] {
        let (directory, home, id, arguments) =
            interrupted_paths(true, false, &["protected", "second"]);
        let mut resumed = Agent::new(
            OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
            Tools::new(directory.path()).unwrap(),
        );
        resumed.enable_sessions(home.path()).unwrap();
        let output = resumed
            .sessions()
            .unwrap()
            .store()
            .output_directory()
            .join(&id);
        let intent = fs::read_dir(output)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "preservation")
            })
            .unwrap();
        let damaged = if valid_json {
            let mut value = crate::json::parse(&fs::read_to_string(&intent).unwrap()).unwrap();
            if let Value::Object(fields) = &mut value
                && let Some(Value::Object(intent)) = fields.get_mut("intent")
                && let Some(Value::Array(entries)) = intent.get_mut("entries")
            {
                entries.truncate(1);
            }
            value.encode()
        } else {
            "{".into()
        };
        fs::write(intent, damaged).unwrap();
        fs::write(directory.path().join("second"), "newer change").unwrap();
        resumed.resume(&id).unwrap();
        assert!(resumed.context.evidence.protection_problem().is_some());
        let status = resumed.tools.protection_status(&Cancellation::default());
        assert_eq!(status.len(), 1);
        assert_eq!(
            status[0].get("registration_incomplete"),
            Some(&Value::Bool(true))
        );
        assert!(
            resumed
                .tools
                .execute("protect", &arguments.encode())
                .get("error")
                .is_some()
        );
        assert!(
            resumed
                .tools
                .execute("bash", r#"{"command":"printf changed > second"}"#)
                .get("error")
                .is_some()
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("second")).unwrap(),
            "newer change"
        );
    }
}

#[test]
fn a_registration_interrupted_before_its_receipt_recovers_the_original_baseline() {
    let (directory, home, id, _) = interrupted_registration(true, false);
    fs::write(directory.path().join("protected"), "later change").unwrap();
    let mut resumed = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    let status = resumed.tools.protection_status(&Cancellation::default());
    assert_eq!(
        status[0].get("state").and_then(Value::as_str),
        Some("violated")
    );
    assert!(resumed.context.evidence.protection_problem().is_some());
    let restored = resumed.tools.execute(
        "protect",
        &Value::object([
            ("action", Value::string("restore")),
            ("paths", Value::Array(vec![Value::string("protected")])),
            (
                "expected_current",
                status[0].get("expected_current").unwrap().clone(),
            ),
        ])
        .encode(),
    );
    assert!(restored.get("error").is_none(), "{restored:?}");
    assert_eq!(
        fs::read(directory.path().join("protected")).unwrap(),
        b"original\r\nlast"
    );
    resumed
        .context
        .validate(&resumed.messages.lock().unwrap())
        .unwrap();
}

#[test]
fn partial_registration_copies_can_recover_only_while_the_declared_original_still_exists() {
    for changed in [false, true] {
        let (directory, home, id, arguments) = interrupted_registration(true, true);
        if changed {
            fs::write(directory.path().join("protected"), "newer change").unwrap();
        }
        let mut resumed = Agent::new(
            OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
            Tools::new(directory.path()).unwrap(),
        );
        resumed.enable_sessions(home.path()).unwrap();
        resumed.resume(&id).unwrap();
        assert!(resumed.context.evidence.protection_problem().is_some());
        resumed.tools.execute("protect", &arguments.encode());
        let status = resumed.tools.protection_status(&Cancellation::default());
        assert_eq!(
            status[0].get("state").and_then(Value::as_str),
            Some(if changed { "unknown" } else { "preserved" })
        );
        if changed {
            assert_eq!(
                fs::read_to_string(directory.path().join("protected")).unwrap(),
                "newer change"
            );
        }
    }
}

#[test]
fn interruption_before_intent_creation_stays_unknown_and_blocks_project_mutations() {
    let (directory, home, id, arguments) = interrupted_registration(false, false);
    fs::write(directory.path().join("protected"), "newer change").unwrap();
    let mut resumed = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert!(resumed.context.evidence.protection_problem().is_some());
    assert!(
        resumed
            .tools
            .execute("bash", r#"{"command":"printf changed > protected"}"#)
            .get("error")
            .is_some()
    );
    assert!(
        resumed
            .tools
            .execute("write", r#"{"path":"protected","content":"changed"}"#)
            .get("error")
            .is_some()
    );
    let result = resumed.tools.execute("protect", &arguments.encode());
    assert!(result.get("error").is_some(), "{result:?}");
    let status = resumed.tools.protection_status(&Cancellation::default());
    assert_eq!(status.len(), 1);
    assert_eq!(
        status[0].get("state").and_then(Value::as_str),
        Some("unknown")
    );
    let release = Value::object([
        ("action", Value::string("release")),
        (
            "paths",
            Value::Array(vec![status[0].get("path").unwrap().clone()]),
        ),
        (
            "reason",
            Value::string("Explicit newer user decision accepting the missing original"),
        ),
    ]);
    assert!(
        resumed
            .tools
            .execute("protect", &release.encode())
            .get("error")
            .is_some()
    );
    resumed.prepare_turn("Discard the interrupted registration whose original version is unavailable. I accept that uncertainty; protect the current file as the new baseline.").unwrap();
    assert!(
        resumed
            .tools
            .execute("protect", &release.encode())
            .get("error")
            .is_none()
    );
    assert!(
        resumed
            .tools
            .execute("protect", &arguments.encode())
            .get("error")
            .is_none()
    );
    assert_eq!(
        resumed.tools.protection_status(&Cancellation::default())[0]
            .get("state")
            .and_then(Value::as_str),
        Some("preserved")
    );
    assert_eq!(
        fs::read(directory.path().join("protected")).unwrap(),
        b"newer change"
    );
}

#[test]
fn recovered_intent_retains_deleted_files_without_reexpanding_the_live_scope() {
    let (directory, home, id, _) = interrupted_registration(true, false);
    fs::remove_file(directory.path().join("protected")).unwrap();
    let mut resumed = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    let status = resumed.tools.protection_status(&Cancellation::default());
    assert_eq!(status.len(), 1);
    assert_eq!(
        status[0].get("state").and_then(Value::as_str),
        Some("violated")
    );
    assert_eq!(
        status[0].get("expected_current").and_then(Value::as_str),
        Some("missing")
    );
    let result = resumed.tools.execute(
        "protect",
        &Value::object([
            ("action", Value::string("restore")),
            ("paths", Value::Array(vec![Value::string("protected")])),
            ("expected_current", Value::string("missing")),
        ])
        .encode(),
    );
    assert!(result.get("error").is_none(), "{result:?}");
    assert_eq!(
        fs::read(directory.path().join("protected")).unwrap(),
        b"original\r\nlast"
    );
}
