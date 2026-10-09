use super::*;

#[test]
fn provider_regression_unbounded_offsets_have_no_platform_sized_schema_maximum() {
    let tools = definitions();
    let read = tools.as_array().unwrap()[0].get("function").unwrap();
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
