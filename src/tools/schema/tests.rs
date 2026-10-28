use super::*;

#[test]
fn tool_catalog_and_encoded_size_are_reused() {
    assert!(std::ptr::eq(definitions(), definitions()));
    assert_eq!(encoded_size(), definitions().encode().len());
}

#[test]
fn every_tool_advertises_optional_parallel_boolean() {
    let tools = definitions().as_array().unwrap();
    assert_eq!(tools.len(), 15);
    for tool in tools {
        let name = tool.get("name").and_then(Value::as_str).unwrap();
        let parameters = tool.get("parameters").unwrap();
        let parallel = parameters
            .get("properties")
            .unwrap()
            .get("parallel")
            .unwrap();
        assert_eq!(
            parallel.get("type"),
            Some(&Value::string("boolean")),
            "{name}"
        );
        assert!(
            parallel
                .get("description")
                .and_then(Value::as_str)
                .is_some()
        );
        assert!(
            !parameters
                .get("required")
                .and_then(Value::as_array)
                .unwrap()
                .contains(&Value::string("parallel")),
            "{name}"
        );
    }
}

#[test]
fn malformed_parallel_metadata_does_not_execute_a_write() {
    let directory = crate::test_support::Directory::new();
    let tools = crate::tools::Tools::new(directory.path()).unwrap();
    for value in [Value::string("true"), Value::Null] {
        let result = tools.execute(
            "write",
            &Value::object([
                ("path", Value::string("must-not-exist.txt")),
                ("content", Value::string("not written")),
                ("parallel", value),
            ])
            .encode(),
        );
        assert_eq!(
            result.get("error").and_then(Value::as_str),
            Some("parallel must be a boolean")
        );
        assert!(!directory.path().join("must-not-exist.txt").exists());
    }
}

#[test]
fn work_and_description_are_optional_only_for_routine_tools() {
    for tool in definitions().as_array().unwrap() {
        let name = tool.get("name").and_then(Value::as_str).unwrap();
        let parameters = tool.get("parameters").unwrap();
        let properties = parameters.get("properties").unwrap();
        let required = parameters
            .get("required")
            .and_then(Value::as_array)
            .unwrap();
        for field in ["work", "description"] {
            assert!(!required.contains(&Value::string(field)), "{name}: {field}");
        }
        assert_eq!(
            parameters.get("additionalProperties"),
            Some(&Value::Bool(false)),
            "{name}"
        );
        if matches!(
            name,
            "read" | "bash" | "process" | "web" | "browser" | "exec" | "list" | "search" | "map"
        ) {
            let work = properties.get("work").unwrap();
            assert_eq!(work.get("type"), Some(&Value::string("string")), "{name}");
            assert!(work.get("description").and_then(Value::as_str).is_some());
            let description = properties.get("description").unwrap();
            assert_eq!(
                description.get("type"),
                Some(&Value::string("string")),
                "{name}"
            );
            assert!(
                description
                    .get("description")
                    .and_then(Value::as_str)
                    .is_some(),
                "{name}"
            );
        } else if name == "agent" {
            // Agent events join Work chapters by label; their rows describe themselves.
            let work = properties.get("work").unwrap();
            assert_eq!(work.get("type"), Some(&Value::string("string")));
            assert!(properties.get("description").is_none());
        } else {
            assert!(properties.get("work").is_none(), "{name}");
            assert!(properties.get("description").is_none(), "{name}");
        }
    }
}

#[test]
fn command_and_process_budgets_advertise_the_same_bounded_preview_contract() {
    for tool in definitions().as_array().unwrap().iter().filter(|tool| {
        matches!(
            tool.get("name").and_then(Value::as_str),
            Some("bash" | "process" | "exec")
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
fn edit_schema_advertises_only_ordered_replacements() {
    let tools = definitions();
    let edit = &tools.as_array().unwrap()[2];
    let parameters = edit.get("parameters").unwrap();
    assert_eq!(
        parameters.get("required"),
        Some(&Value::Array(vec![
            Value::string("path"),
            Value::string("edits")
        ]))
    );
    assert_eq!(
        parameters.get("additionalProperties"),
        Some(&Value::Bool(false))
    );
    let properties = parameters.get("properties").unwrap();
    assert!(properties.get("old_text").is_none());
    assert!(properties.get("new_text").is_none());
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
    assert_eq!(functions.len(), 15);
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
fn map_is_appended_as_a_bounded_routine_tool() {
    let tools = definitions().as_array().unwrap();
    assert_eq!(tools[11].get("name").and_then(Value::as_str), Some("map"));
    let parameters = tools[11].get("parameters").unwrap();
    assert_eq!(parameters.get("required"), Some(&Value::Array(vec![])));
    let properties = parameters.get("properties").unwrap();
    let actions = properties.get("action").unwrap().get("enum").unwrap();
    assert_eq!(
        actions,
        &Value::Array(["overview", "remember", "forget"].map(Value::string).into())
    );
    assert_eq!(
        properties.get("offset").unwrap().get("minimum"),
        Some(&Value::number(1))
    );
    let limit = properties.get("limit").unwrap();
    assert_eq!(limit.get("minimum"), Some(&Value::number(1)));
    assert_eq!(limit.get("maximum"), Some(&Value::number(1000)));
    let sources = properties.get("sources").unwrap();
    assert_eq!(sources.get("type"), Some(&Value::string("array")));
    let source = sources.get("items").unwrap();
    assert_eq!(
        source.get("additionalProperties"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        source.get("required"),
        Some(&Value::Array(vec![Value::string("path")]))
    );
    assert_eq!(
        source
            .get("properties")
            .unwrap()
            .get("line")
            .unwrap()
            .get("minimum"),
        Some(&Value::number(1))
    );
    for name in ["path", "query", "id", "title", "summary"] {
        assert_eq!(
            properties.get(name).unwrap().get("type"),
            Some(&Value::string("string")),
            "{name}"
        );
    }
    for name in ["include_hidden", "include_ignored"] {
        assert_eq!(
            properties.get(name).unwrap().get("type"),
            Some(&Value::string("boolean")),
            "{name}"
        );
    }
}

#[test]
fn exec_is_appended_without_moving_browser_and_advertises_structured_terminal_input() {
    let tools = definitions().as_array().unwrap();
    assert_eq!(
        tools[7].get("name").and_then(Value::as_str),
        Some("browser")
    );
    let exec = &tools[8];
    assert_eq!(exec.get("name").and_then(Value::as_str), Some("exec"));
    let parameters = exec.get("parameters").unwrap();
    assert_eq!(
        parameters.get("required"),
        Some(&Value::Array(vec![Value::string("executable")]))
    );
    let fields = parameters.get("properties").unwrap();
    assert_eq!(
        fields
            .get("args")
            .unwrap()
            .get("items")
            .unwrap()
            .get("type"),
        Some(&Value::string("string"))
    );
    assert_eq!(
        fields.get("tty").unwrap().get("type"),
        Some(&Value::string("boolean"))
    );
    for field in ["cols", "rows"] {
        assert_eq!(
            fields.get(field).unwrap().get("minimum"),
            Some(&Value::number(1))
        );
        assert_eq!(
            fields.get(field).unwrap().get("maximum"),
            Some(&Value::number(1000))
        );
    }
    assert_eq!(
        fields
            .get("env")
            .unwrap()
            .get("additionalProperties")
            .unwrap()
            .get("type"),
        Some(&Value::Array(vec![
            Value::string("string"),
            Value::string("null")
        ]))
    );
    let process = tools
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some("process"))
        .unwrap();
    let fields = process
        .get("parameters")
        .unwrap()
        .get("properties")
        .unwrap();
    let actions = fields
        .get("action")
        .unwrap()
        .get("enum")
        .unwrap()
        .as_array()
        .unwrap();
    for action in ["write", "resize", "interrupt"] {
        assert!(actions.contains(&Value::string(action)));
    }
    assert!(
        fields
            .get("key")
            .unwrap()
            .get("enum")
            .unwrap()
            .as_array()
            .unwrap()
            .contains(&Value::string("Ctrl+C"))
    );
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

#[test]
fn browser_advertises_viewport_bounds_and_log_paging() {
    let tools = definitions();
    let browser = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some("browser"))
        .unwrap();
    let properties = browser
        .get("parameters")
        .unwrap()
        .get("properties")
        .unwrap();
    for (name, maximum) in [("width", 10_000), ("height", 10_000), ("limit", 200)] {
        let field = properties.get(name).unwrap();
        assert_eq!(field.get("minimum"), Some(&Value::number(1)), "{name}");
        assert_eq!(
            field.get("maximum"),
            Some(&Value::number(maximum)),
            "{name}"
        );
    }
    let filters = properties.get("filter").unwrap().get("enum").unwrap();
    assert_eq!(
        filters,
        &Value::Array(
            ["default", "console", "errors", "network", "all"]
                .map(Value::string)
                .into()
        )
    );
    assert_eq!(
        properties.get("cursor").unwrap().get("type"),
        Some(&Value::string("string"))
    );
    let description = browser.get("description").and_then(Value::as_str).unwrap();
    for phrase in ["page viewport", "role, name", "next_cursor", "missed"] {
        assert!(description.contains(phrase), "{phrase}");
    }
}

#[test]
fn discovery_tools_append_to_the_existing_catalog_with_bounded_arguments() {
    let tools = definitions().as_array().unwrap();
    let names = tools
        .iter()
        .map(|tool| tool.get("name").and_then(Value::as_str).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "read",
            "write",
            "edit",
            "bash",
            "process",
            "checkpoint",
            "web",
            "browser",
            "exec",
            "list",
            "search",
            "map",
            "agent",
            "todo_list",
            "ask_user"
        ]
    );
    for (name, required, limit_max, context_max) in [
        ("list", Vec::new(), 2_000, None),
        ("search", vec![Value::string("query")], 1_000, Some(10)),
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
            .unwrap();
        let parameters = tool.get("parameters").unwrap();
        assert_eq!(parameters.get("required"), Some(&Value::Array(required)));
        assert_eq!(
            parameters.get("additionalProperties"),
            Some(&Value::Bool(false))
        );
        let fields = parameters.get("properties").unwrap();
        for field in ["path", "pattern", "work", "description"] {
            assert_eq!(
                fields.get(field).unwrap().get("type"),
                Some(&Value::string("string")),
                "{name}: {field}"
            );
        }
        for field in ["include_hidden", "include_ignored"] {
            assert_eq!(
                fields.get(field).unwrap().get("type"),
                Some(&Value::string("boolean")),
                "{name}: {field}"
            );
        }
        for field in ["offset", "limit"] {
            let value = fields.get(field).unwrap();
            assert_eq!(value.get("type"), Some(&Value::string("integer")));
            assert_eq!(value.get("minimum"), Some(&Value::number(1)));
            if field == "limit" {
                assert_eq!(value.get("maximum"), Some(&Value::number(limit_max)));
            } else {
                assert!(value.get("maximum").is_none());
            }
        }
        if let Some(context_max) = context_max {
            assert!(fields.get("recursive").is_none());
            assert_eq!(
                fields.get("case_sensitive").unwrap().get("type"),
                Some(&Value::string("boolean"))
            );
            assert_eq!(
                fields.get("context").unwrap().get("maximum"),
                Some(&Value::number(context_max))
            );
            assert_eq!(
                fields.get("query").unwrap().get("type"),
                Some(&Value::string("string"))
            );
        } else {
            assert!(fields.get("query").is_none());
            assert_eq!(
                fields.get("recursive").unwrap().get("type"),
                Some(&Value::string("boolean"))
            );
        }
    }
}
