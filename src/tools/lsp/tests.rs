use super::*;
use crate::test_support::Directory;

#[test]
fn tool_status_reads_saved_entries_without_executing_a_program() {
    let home = Directory::new();
    let workspace = Directory::new();
    let mut profile = Profile::draft_guided_rust();
    profile.executable = "definitely-missing-jecode-lsp-test-server".into();
    Store::new(home.path().to_path_buf())
        .save(profile.clone())
        .unwrap();
    let mut tools = Tools::new(workspace.path()).unwrap();
    tools.configure_lsp(home.path(), Redactor::empty()).unwrap();
    let result = tools.execute("lsp", r#"{"action":"status"}"#);
    assert!(result.get("error").is_none(), "{}", result.pretty());
    assert!(result.encode().contains(&profile.id));
    let saved = &result.get("servers").and_then(Value::as_array).unwrap()[0];
    assert_eq!(
        saved.get("name").and_then(Value::as_str),
        Some(profile.name.as_str())
    );
    assert_eq!(
        saved
            .get("extensions")
            .and_then(|value| value.get("rs"))
            .and_then(Value::as_str),
        Some("rust")
    );
    assert!(tools.lsp_runtime().statuses().is_empty());
    assert!(
        tools
            .access("lsp", r#"{"action":"diagnostics","path":"src/main.rs"}"#)
            .is_none()
    );
}

#[test]
fn disabled_server_and_invalid_arguments_cannot_launch() {
    let home = Directory::new();
    let workspace = Directory::new();
    let mut profile = Profile::draft_guided_rust();
    profile.enabled = false;
    profile.executable = "definitely-missing-jecode-lsp-test-server".into();
    Store::new(home.path().to_path_buf()).save(profile).unwrap();
    std::fs::write(workspace.path().join("main.rs"), "fn main() {}\n").unwrap();
    let mut tools = Tools::new(workspace.path()).unwrap();
    tools.configure_lsp(home.path(), Redactor::empty()).unwrap();
    let result = tools.execute(
        "lsp",
        r#"{"action":"definition","path":"main.rs","line":1,"column":1}"#,
    );
    assert!(
        result.encode().contains("off") || result.encode().contains("disabled"),
        "{}",
        result.pretty()
    );
    assert!(tools.lsp_runtime().statuses().is_empty());
    for invalid in [
        r#"{"action":"definition","path":"main.rs","line":0,"column":1}"#,
        r#"{"action":"references","path":"main.rs","line":1,"column":1,"include_declaration":"true"}"#,
        r#"{"action":"diagnostics","path":"main.rs","server":3}"#,
    ] {
        assert!(tools.execute("lsp", invalid).get("error").is_some());
    }
}

#[test]
fn oversized_results_retain_complete_data_and_independent_completeness() {
    let directory = Directory::new();
    let outputs = crate::output::Store::new(directory.path().join("outputs"), Redactor::empty());
    let value = Value::object([
        ("status", Value::string("done")),
        ("complete", Value::Bool(true)),
        (
            "locations",
            Value::Array(
                (0..500)
                    .map(|index| {
                        Value::object([(
                            "path",
                            Value::string(format!("{}-{index}.rs", "long".repeat(100))),
                        )])
                    })
                    .collect(),
            ),
        ),
    ]);
    let bounded = bounded::bound(value, &outputs);
    assert_eq!(bounded.get("complete"), Some(&Value::Bool(true)));
    assert_eq!(bounded.get("truncated"), Some(&Value::Bool(true)));
    assert!(bounded.get("result_file").and_then(Value::as_str).is_some());
    assert_eq!(
        bounded.get("total_locations").and_then(Value::as_usize),
        Some(500)
    );
    assert!(bounded.encode().len() <= crate::tools::OUTPUT_LIMIT);
}

#[test]
fn environment_values_are_never_in_status_or_presentation_names() {
    let home = Directory::new();
    let workspace = Directory::new();
    let mut profile = Profile::draft_guided_rust();
    profile.environment.insert(
        "JECODE_FAKE_SECRET".into(),
        "isolated-lsp-secret-value".into(),
    );
    profile.name = "Server isolated-lsp-secret-value".into();
    Store::new(home.path().to_path_buf()).save(profile).unwrap();
    let mut tools = Tools::new(workspace.path()).unwrap();
    tools.configure_lsp(home.path(), Redactor::empty()).unwrap();
    let result = tools.execute("lsp", r#"{"action":"status"}"#);
    assert!(!result.encode().contains("isolated-lsp-secret-value"));
}

#[test]
fn escaped_server_output_stays_within_the_actual_encoded_budget() {
    let directory = Directory::new();
    let outputs = crate::output::Store::new(directory.path().join("outputs"), Redactor::empty());
    let value = Value::object([
        ("status", Value::string("ready")),
        ("complete", Value::Bool(false)),
        ("root", Value::string("\\long".repeat(20_000))),
        (
            "diagnostics",
            Value::Array(vec![Value::object([(
                "message",
                Value::string("\0\\\"🦀".repeat(100_000)),
            )])]),
        ),
    ]);
    let bounded = bounded::bound(value, &outputs);
    assert!(bounded.encode().len() <= crate::tools::OUTPUT_LIMIT);
    assert!(bounded.get("result_file").is_some());
    assert_eq!(bounded.get("complete"), Some(&Value::Bool(false)));
}
