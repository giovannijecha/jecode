use super::*;
use crate::redact::Redactor;
use crate::test_support::Directory;
use crate::tools::Tools;

fn tools(directory: &Directory, redactor: Redactor) -> Tools {
    let mut tools = Tools::new(directory.path()).unwrap();
    tools.configure_output(directory.path().join("logs"), redactor);
    tools
}

fn bash(tools: &Tools, command: &str, limit: Option<usize>) -> Value {
    let mut arguments = Value::object([("command", Value::string(command))]);
    if let (Some(limit), Value::Object(fields)) = (limit, &mut arguments) {
        fields.insert("output_limit_bytes".into(), Value::number(limit));
    }
    tools.execute("bash", &arguments.encode())
}

#[test]
fn foreground_preview_keeps_head_and_tail_and_full_saved_output() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let result = bash(&tools, "printf HEAD; printf '%20000s' x; printf TAIL", None);
    let preview = result.get("stdout").and_then(Value::as_str).unwrap();
    assert!(preview.starts_with("HEAD"));
    assert!(preview.ends_with("TAIL"));
    assert!(preview.contains("saved bytes omitted"));
    assert_eq!(result.get("stdout_bytes"), Some(&Value::number(20008)));
    assert_eq!(
        result.get("output_limit_bytes"),
        Some(&Value::number(16 * 1024))
    );
    assert_eq!(result.get("stdout_truncated"), Some(&Value::Bool(true)));
    let reread = tools.execute(
        "read",
        &Value::object([
            ("path", result.get("stdout_file").unwrap().clone()),
            ("byte_offset", Value::number(0)),
        ])
        .encode(),
    );
    let saved = reread.get("content").and_then(Value::as_str).unwrap();
    assert_eq!(saved.len(), 20008);
    assert!(saved.starts_with("HEAD") && saved.ends_with("TAIL"));
}

#[test]
fn chosen_budget_can_exceed_the_old_sixty_four_kib_cap() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let result = bash(&tools, "printf '%70000s' x", Some(70000));
    assert_eq!(
        result.get("output_limit_bytes"),
        Some(&Value::number(70000))
    );
    assert_eq!(result.get("stdout_truncated"), Some(&Value::Bool(false)));
    assert_eq!(
        result.get("stdout").and_then(Value::as_str).unwrap().len(),
        70000
    );
}

#[test]
fn preview_respects_utf8_boundaries_and_masks_credentials() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::new("fixture-secret".into()));
    let result = bash(
        &tools,
        "printf 'é fixture-secret'; printf '%20000s' x; printf '🦀'",
        Some(9),
    );
    let preview = result.get("stdout").and_then(Value::as_str).unwrap();
    assert!(preview.starts_with("é"));
    assert!(preview.ends_with("🦀"));
    assert!(!preview.contains('�'));
    assert!(!preview.contains("fixture-secret"));
    let path = tools
        .outputs
        .resolve(result.get("stdout_file").unwrap().as_str().unwrap())
        .unwrap();
    let saved = std::fs::read_to_string(path).unwrap();
    assert!(saved.contains("[redacted]"));
    assert!(!saved.contains("fixture-secret"));
}

#[test]
fn invalid_output_budget_rejects_bash_before_launch() {
    let directory = Directory::new();
    let tools = tools(&directory, Redactor::empty());
    let result = bash(&tools, "printf touched > touched", Some(0));
    assert!(
        result
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("output_limit_bytes")
    );
    assert!(!directory.path().join("touched").exists());
}
