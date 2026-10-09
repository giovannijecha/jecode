use crate::json::Value;

pub fn tool_call(id: &str, name: &str, arguments: Value) -> Value {
    Value::object([
        ("id", Value::string(id)),
        ("type", Value::string("function")),
        (
            "function",
            Value::object([
                ("name", Value::string(name)),
                ("arguments", Value::string(arguments.encode())),
            ]),
        ),
    ])
}

pub fn completion(text: &str, calls: Vec<Value>) -> Value {
    Value::object([(
        "choices",
        Value::Array(vec![Value::object([
            (
                "finish_reason",
                Value::string(if calls.is_empty() {
                    "stop"
                } else {
                    "tool_calls"
                }),
            ),
            (
                "message",
                Value::object([
                    ("role", Value::string("assistant")),
                    ("content", Value::string(text)),
                    ("tool_calls", Value::Array(calls)),
                    (
                        "reasoning_details",
                        Value::Array(vec![Value::object([
                            ("type", Value::string("reasoning.encrypted")),
                            ("data", Value::string("fixture-reasoning")),
                        ])]),
                    ),
                ]),
            ),
        ])]),
    )])
}
