use super::*;

#[test]
fn a_failed_result_write_preserves_read_results_and_the_resume_index() {
    let project = Directory::new();
    let names = ["a.txt", "b.txt", "c.txt", "d.txt"];
    for name in names {
        fs::write(
            project.path().join(name),
            format!("marker-{name}\n{}\n", "x".repeat(50_000)),
        )
        .unwrap();
    }
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(project.path()).unwrap(),
    );
    let arguments = paths(&names);
    let call = ToolCall {
        id: "failed-save".into(),
        name: "read".into(),
        arguments: arguments.encode(),
    };
    let result = agent.read_paths_with(
        &call,
        &arguments,
        &crate::tools::Begun::default(),
        |first| {
            let mut file = Overflow::start(agent.tools.outputs(), first)?;
            let path = agent.tools.outputs().resolve(&file.logs.stdout_ref)?;
            file.logs.stdout = output::Spool::read_only_fixture(&path);
            Ok(file)
        },
    );
    assert!(
        result
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("Could not save read results")
    );
    let files = result
        .get("files")
        .and_then(Value::as_array)
        .expect("completed reads must survive a failed result write");
    assert_eq!(files.len(), 2);
    for (file, name) in files.iter().zip(names) {
        assert_eq!(file.get("path").and_then(Value::as_str), Some(name));
        assert!(
            file.get("content")
                .and_then(Value::as_str)
                .unwrap()
                .contains(&format!("marker-{name}"))
        );
    }
    assert_eq!(result.get("paths_processed"), Some(&Value::number(2)));
    assert_eq!(result.get("next_path_index"), Some(&Value::number(2)));
    assert_eq!(result.get("files_read"), Some(&Value::number(2)));
    assert!(read(&agent, paths(&names[2..])).get("error").is_none());
}

#[test]
fn a_partial_saved_file_keeps_its_reference_and_completed_prefix() {
    let project = Directory::new();
    let tools = Tools::new(project.path()).unwrap();
    let entry = |path| {
        Value::object([
            ("path", Value::string(path)),
            ("content", Value::string(path)),
        ])
    };
    let mut file = Overflow::start(tools.outputs(), 1).unwrap();
    file.push(&entry("saved-b")).unwrap();
    let path = tools.outputs().resolve(&file.logs.stdout_ref).unwrap();
    file.logs.stdout = output::Spool::read_only_fixture(&path);
    let error = file.push(&entry("inline-c")).unwrap_err();
    let result = batch_result(
        vec![entry("inline-a"), entry("inline-c")],
        (3, 0, 3),
        std::collections::BTreeSet::new(),
        Some(file.saved()),
        Some(ReadFailure {
            error,
            processed: 3,
            total: 4,
        }),
    );
    assert_eq!(result.get("saved_files"), Some(&Value::number(1)));
    assert_eq!(
        result.get("result_file_incomplete"),
        Some(&Value::Bool(true))
    );
    assert_eq!(result.get("next_path_index"), Some(&Value::number(3)));
    let reference = result.get("result_file").and_then(Value::as_str).unwrap();
    assert!(
        fs::read_to_string(tools.outputs().resolve(reference).unwrap())
            .unwrap()
            .contains("saved-b")
    );
    assert_eq!(
        result.get("files").and_then(Value::as_array).unwrap().len(),
        2
    );
}
