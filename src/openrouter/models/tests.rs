use super::*;
#[test]
fn catalog_filters_tools_and_uses_declared_efforts_without_inventing_support() {
    let value = Value::object([(
        "data",
        Value::Array(vec![
            Value::object([
                ("id", Value::string("no-tools")),
                ("supported_parameters", Value::Array(vec![])),
            ]),
            Value::object([
                ("id", Value::string("plain")),
                (
                    "supported_parameters",
                    Value::Array(vec![Value::string("tools")]),
                ),
            ]),
            Value::object([
                ("id", Value::string("reasoning")),
                (
                    "supported_parameters",
                    Value::Array(vec![Value::string("tools")]),
                ),
                (
                    "reasoning",
                    Value::object([
                        (
                            "supported_efforts",
                            Value::Array(vec![
                                Value::string("high"),
                                Value::string("none"),
                                Value::string("future-value"),
                            ]),
                        ),
                        ("mandatory", Value::Bool(true)),
                    ]),
                ),
            ]),
        ]),
    )]);
    let models = parse_models(value, usize::MAX).unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].efforts, [Effort::Default]);
    assert_eq!(models[1].efforts, [Effort::Default, Effort::High]);
    let value = Value::object([(
        "reasoning",
        Value::object([("supported_efforts", Value::Null)]),
    )]);
    assert_eq!(efforts(&value).len(), 8);
}
