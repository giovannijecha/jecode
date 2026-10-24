use super::*;

#[test]
fn tool_catalog_and_encoded_size_are_reused() {
    assert!(std::ptr::eq(definitions(), definitions()));
    assert_eq!(encoded_size(), definitions().encode().len());
}

#[test]
fn shell_and_process_budgets_advertise_the_same_bounded_preview_contract() {
    for tool in definitions().as_array().unwrap().iter().filter(|tool| {
        matches!(
            tool.get("name").and_then(Value::as_str),
            Some("bash" | "process")
        )
    }) {
        let budget = tool
            .get("parameters")
            .unwrap()
            .get("properties")
            .unwrap()
            .get("output_limit_bytes")
            .unwrap();
        assert_eq!(budget.get("minimum"), Some(&Value::number(1)));
        assert_eq!(budget.get("maximum"), Some(&Value::number(256 * 1024)));
        if tool.get("name").and_then(Value::as_str) == Some("process") {
            let mode = tool
                .get("parameters")
                .unwrap()
                .get("properties")
                .unwrap()
                .get("output_mode")
                .unwrap();
            assert_eq!(
                mode.get("enum"),
                Some(&Value::Array(["next", "tail"].map(Value::string).into()))
            );
        }
    }
}

#[test]
fn edit_schema_advertises_ordered_batch_and_legacy_fields() {
    let tools = definitions();
    let edit = &tools.as_array().unwrap()[2];
    let parameters = edit.get("parameters").unwrap();
    assert_eq!(
        parameters.get("required"),
        Some(&Value::Array(vec![Value::string("path")]))
    );
    let properties = parameters.get("properties").unwrap();
    assert!(properties.get("old_text").is_some());
    assert!(properties.get("new_text").is_some());
    let edits = properties.get("edits").unwrap();
    assert_eq!(edits.get("minItems"), Some(&Value::number(1)));
    let items = edits.get("items").unwrap();
    assert_eq!(items.get("additionalProperties"), Some(&Value::Bool(false)));
    assert_eq!(
        items.get("required"),
        Some(&Value::Array(vec![
            Value::string("old_text"),
            Value::string("new_text")
        ]))
    );
}

#[test]
fn provider_regression_unbounded_offsets_have_no_platform_sized_schema_maximum() {
    let tools = definitions();
    let read = &tools.as_array().unwrap()[0];
    let properties = read.get("parameters").unwrap().get("properties").unwrap();
    for name in ["offset", "byte_offset"] {
        let parameter = properties.get(name).unwrap();
        assert!(
            parameter.get("maximum").is_none(),
            "{name} advertises a platform-sized maximum that OpenAI rejects"
        );
        assert_eq!(
            parameter.get("type").and_then(Value::as_str),
            Some("integer")
        );
    }
    assert_eq!(
        properties.get("limit").unwrap().get("maximum"),
        Some(&Value::number(2000))
    );
}

#[test]
fn web_and_browser_advertise_local_actions_and_signed_scroll() {
    let tools = definitions();
    let functions = tools.as_array().unwrap();
    assert_eq!(functions.len(), 8);
    assert!(functions.iter().all(|tool| tool.get("function").is_none()));
    for (name, action) in [("web", "crawl"), ("browser", "cdp")] {
        let function = functions
            .iter()
            .find(|function| function.get("name").and_then(Value::as_str) == Some(name))
            .unwrap();
        let properties = function
            .get("parameters")
            .unwrap()
            .get("properties")
            .unwrap();
        let actions = properties
            .get("action")
            .unwrap()
            .get("enum")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(actions.contains(&Value::string(action)));
        assert_eq!(
            function
                .get("parameters")
                .unwrap()
                .get("additionalProperties"),
            Some(&Value::Bool(false))
        );
    }
    let browser = &functions[7];
    let scroll = browser
        .get("parameters")
        .unwrap()
        .get("properties")
        .unwrap()
        .get("delta_y")
        .unwrap();
    assert_eq!(scroll.get("type").and_then(Value::as_str), Some("integer"));
    assert!(scroll.get("minimum").is_none());
}

#[test]
fn bash_contract_distinguishes_wait_from_execution_deadline() {
    let tools = definitions();
    let bash = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some("bash"))
        .unwrap();
    let description = bash.get("description").and_then(Value::as_str).unwrap();
    assert!(description.contains("Bash on every host, including Windows"));
    let properties = bash.get("parameters").unwrap().get("properties").unwrap();
    let timeout = properties.get("timeout_seconds").unwrap();
    let wait = properties.get("wait_ms").unwrap();
    assert_eq!(timeout.get("minimum"), Some(&Value::number(1)));
    assert_eq!(wait.get("minimum"), Some(&Value::number(0)));
    assert!(
        timeout
            .get("description")
            .and_then(Value::as_str)
            .unwrap()
            .contains("omitted means no deadline")
    );
    assert!(
        wait.get("description")
            .and_then(Value::as_str)
            .unwrap()
            .contains("does not end the process")
    );
}
