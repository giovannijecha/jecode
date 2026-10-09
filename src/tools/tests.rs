use super::*;
use crate::test_support::Directory;

#[test]
fn saved_large_output_can_be_read_in_full_and_cannot_be_modified_by_file_tools() {
    let workspace = Directory::new();
    let home = Directory::new();
    let mut tools = Tools::new(workspace.path()).unwrap();
    tools.configure_output(
        home.path().join("logs"),
        crate::redact::Redactor::new("fixture-secret".into()),
    );
    let result = tools.execute(
        "bash",
        &Value::object([
            (
                "command",
                Value::string("printf '%70000s' x; printf 'fixture-secret' >&2"),
            ),
            ("timeout_seconds", Value::number(601)),
        ])
        .encode(),
    );
    assert_eq!(result.get("exit_code"), Some(&Value::number(0)));
    assert!(
        result
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap()
            .ends_with('x')
    );
    let path = result.get("stdout_file").and_then(Value::as_str).unwrap();
    let mut offset = 0;
    let mut restored = String::new();
    loop {
        let page = tools.execute(
            "read",
            &Value::object([
                ("path", Value::string(path)),
                ("byte_offset", Value::number(offset)),
            ])
            .encode(),
        );
        restored.push_str(page.get("content").and_then(Value::as_str).unwrap());
        match page.get("next_byte_offset").and_then(Value::as_usize) {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
    }
    assert_eq!(restored, format!("{}x", " ".repeat(69999)));
    let stderr = result.get("stderr_file").and_then(Value::as_str).unwrap();
    let page = tools.execute(
        "read",
        &Value::object([("path", Value::string(stderr))]).encode(),
    );
    assert_eq!(
        page.get("content").and_then(Value::as_str),
        Some("1: [redacted]")
    );
    for name in ["write", "edit"] {
        assert!(
            tools
                .execute(
                    name,
                    &Value::object([
                        ("path", Value::string(path)),
                        ("content", Value::string("overwrite")),
                        ("old_text", Value::string("x")),
                        ("new_text", Value::string("y"))
                    ])
                    .encode()
                )
                .get("error")
                .is_some()
        );
    }
    assert_eq!(
        std::fs::read(tools.outputs.resolve(path).unwrap())
            .unwrap()
            .len(),
        70000
    );
}

#[test]
fn invalid_calls_return_errors_without_side_effects() {
    let directory = Directory::new();
    let tools = Tools::new(directory.path()).unwrap();
    for (name, arguments) in [
        ("unknown", "{}"),
        ("write", "not json"),
        ("write", "[]"),
        ("write", "{}"),
        ("read", r#"{"path":"missing","offset":0}"#),
    ] {
        assert!(tools.execute(name, arguments).get("error").is_some());
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn session_output_configuration_switches_owner_and_keeps_prior_refs_readable() {
    let workspace = Directory::new();
    let home = Directory::new();
    let root = home.path().join("outputs");
    let mut tools = Tools::new(workspace.path()).unwrap();
    tools
        .configure_session_output(root.clone(), "1-1-1", crate::redact::Redactor::empty())
        .unwrap();
    assert!(!root.exists());
    let first = tools.outputs.create().unwrap();
    let reference = first.stdout_ref.clone();
    drop(first);
    tools
        .configure_session_output(root.clone(), "2-2-2", crate::redact::Redactor::empty())
        .unwrap();
    assert!(tools.outputs.resolve(&reference).is_ok());
    let second = tools.outputs.create().unwrap();
    assert!(second.stdout_ref.starts_with("output:2-2-2:"));
    drop(second);
    let prior = crate::output::Store::for_session(
        root,
        workspace.path().to_path_buf(),
        "1-1-1",
        crate::redact::Redactor::empty(),
    )
    .unwrap();
    tools.configure_output_store(prior);
    assert!(tools.outputs.resolve(&reference).is_ok());
}

#[test]
fn attachment_references_read_text_pages_and_describe_other_kinds() {
    let workspace = Directory::new();
    let home = Directory::new();
    let outside = Directory::new();
    let mut tools = Tools::new(workspace.path()).unwrap();
    let read = |tools: &Tools, path: &str, offset: usize| {
        tools.execute(
            "read",
            &Value::object([
                ("path", Value::string(path)),
                ("byte_offset", Value::number(offset)),
            ])
            .encode(),
        )
    };
    assert!(
        read(&tools, "attachment:att-1-2-3", 0)
            .get("error")
            .is_some()
    );
    let pool = crate::attachments::Pool::new(home.path().join("attachments"));
    tools.configure_attachments(pool.clone());
    let source = outside.path().join("notes.txt");
    std::fs::write(&source, "outside the workspace").unwrap();
    let text = pool
        .import_file(&source, &crate::cancel::Cancellation::default())
        .unwrap();
    let page = read(&tools, &text.reference(), 8);
    assert_eq!(
        page.get("content").and_then(Value::as_str),
        Some("the workspace")
    );
    assert_eq!(
        page.get("reference").and_then(Value::as_str),
        Some(text.reference().as_str())
    );
    assert!(
        page.get("attached_from")
            .and_then(Value::as_str)
            .unwrap()
            .ends_with("notes.txt")
    );
    let image = pool
        .import_bytes("shot.png", &crate::attachments::tests::png(2, 2))
        .unwrap();
    let shown = read(&tools, &image.reference(), 0);
    assert_eq!(shown.get("view").and_then(Value::as_str), Some("image"));
    assert!(!shown.encode().contains("base64"));
    let binary = pool.import_bytes("blob.bin", &[0, 1, 2, 255]).unwrap();
    let opaque = read(&tools, &binary.reference(), 0);
    assert!(opaque.get("view").is_none());
    assert!(opaque.get("content").is_none());
    assert!(
        opaque
            .get("note")
            .and_then(Value::as_str)
            .unwrap()
            .contains("has not interpreted")
    );
    let local = opaque.get("local_path").and_then(Value::as_str).unwrap();
    assert_eq!(std::fs::read(local).unwrap(), [0, 1, 2, 255]);
    assert!(
        read(&tools, "attachment:../escape", 0)
            .get("error")
            .is_some()
    );
}
