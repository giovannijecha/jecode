use super::*;

#[test]
fn web_and_browser_advertise_local_actions_and_signed_scroll() {
    let tools = definitions();
    let functions = tools.as_array().unwrap();
    assert_eq!(functions.len(), 16);
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
    let browser = &functions[6];
    let scroll = browser
        .get("parameters")
        .unwrap()
        .get("properties")
        .unwrap()
        .get("delta_y")
        .unwrap();
    assert_eq!(scroll.get("type").and_then(Value::as_str), Some("integer"));
    assert!(scroll.get("minimum").is_none());
    let properties = browser
        .get("parameters")
        .unwrap()
        .get("properties")
        .unwrap();
    assert!(
        properties
            .get("action")
            .unwrap()
            .get("enum")
            .unwrap()
            .as_array()
            .unwrap()
            .contains(&Value::string("type"))
    );
    let modifiers = properties.get("modifiers").unwrap();
    assert_eq!(modifiers.get("minimum"), Some(&Value::number(0)));
    assert_eq!(modifiers.get("maximum"), Some(&Value::number(15)));
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
    for (name, maximum) in [("width", 10_000_000), ("height", 10_000_000)] {
        let field = properties.get(name).unwrap();
        assert_eq!(field.get("minimum"), Some(&Value::number(1)), "{name}");
        assert_eq!(
            field.get("maximum"),
            Some(&Value::number(maximum)),
            "{name}"
        );
    }
    let limit = properties.get("limit").unwrap();
    assert_eq!(limit.get("minimum"), Some(&Value::number(1)));
    assert!(limit.get("maximum").is_none());
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
