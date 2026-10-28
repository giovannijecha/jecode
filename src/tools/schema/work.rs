use crate::json::Value;

fn string(description: &str) -> Value {
    Value::object([
        ("type", Value::string("string")),
        ("description", Value::string(description)),
    ])
}

fn object(properties: Value, required: &[&str]) -> Value {
    Value::object([
        ("type", Value::string("object")),
        ("properties", properties),
        (
            "required",
            Value::Array(required.iter().map(|name| Value::string(*name)).collect()),
        ),
        ("additionalProperties", Value::Bool(false)),
    ])
}

pub(super) fn todo() -> Value {
    let status = Value::object([
        ("type", Value::string("string")),
        (
            "enum",
            Value::Array(
                ["pending", "in_progress", "waiting", "done", "cancelled"]
                    .map(Value::string)
                    .into(),
            ),
        ),
    ]);
    let task = object(
        Value::object([
            (
                "id",
                string(
                    "Stable task ID across revisions, such as t1. Do not reuse an ID for different work.",
                ),
            ),
            ("title", string("Short action title.")),
            ("status", status),
            (
                "note",
                string("Optional short context, outcome or reason for waiting/cancellation."),
            ),
            (
                "agent",
                string(
                    "Optional collaborator ID for display only. A returned report does not complete this task.",
                ),
            ),
        ]),
        &["id", "title", "status"],
    );
    super::tool(
        "todo_list",
        "Own the current conversation's plan. get reads it; set atomically replaces the full task list and records a chronological snapshot. Keep existing task IDs when changing title, status or note; use cancelled for work removed by a decision. The model chooses when tasks are done. Collaborators keep separate plans. Calls run in order even with parallel:true.",
        Value::object([
            (
                "action",
                Value::object([
                    ("type", Value::string("string")),
                    (
                        "enum",
                        Value::Array(["get", "set"].map(Value::string).into()),
                    ),
                ]),
            ),
            ("title", string("Plan title; required for set.")),
            (
                "tasks",
                Value::object([
                    ("type", Value::string("array")),
                    ("items", task),
                    ("maxItems", Value::number(40)),
                    (
                        "description",
                        Value::string(
                            "Complete replacement list; required for set. An empty list clears the current tasks.",
                        ),
                    ),
                ]),
            ),
        ]),
        &["action"],
    )
}

pub(super) fn ask_user() -> Value {
    let option = object(
        Value::object([
            ("id", string("Stable option ID in this question.")),
            ("label", string("Short choice label.")),
            (
                "description",
                string("Optional consequence or tradeoff of this choice."),
            ),
            (
                "recommended",
                Value::object([("type", Value::string("boolean"))]),
            ),
        ]),
        &["id", "label"],
    );
    super::tool(
        "ask_user",
        "Ask one focused question and wait for an explicit answer. Options are optional and free text is always available. A recommendation is not an answer. If interactive input is unavailable or the question is dismissed, report that no answer was supplied; do not infer consent. Collaborators cannot ask the user directly. Calls run in order even with parallel:true.",
        Value::object([
            ("title", string("Short question title.")),
            (
                "question",
                string("One question requiring the user's decision or missing information."),
            ),
            (
                "context",
                string("Optional concise context for the decision."),
            ),
            (
                "options",
                Value::object([
                    ("type", Value::string("array")),
                    ("items", option),
                    ("maxItems", Value::number(6)),
                ]),
            ),
        ]),
        &["title", "question"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_tool_schemas_and_child_catalog_have_the_expected_contracts() {
        let todo = todo();
        let ask = ask_user();
        assert_eq!(todo.get("name").and_then(Value::as_str), Some("todo_list"));
        assert_eq!(ask.get("name").and_then(Value::as_str), Some("ask_user"));
        let todo_params = todo.get("parameters").unwrap();
        assert_eq!(
            todo_params
                .get("required")
                .and_then(Value::as_array)
                .unwrap(),
            &[Value::string("action")]
        );
        let ask_params = ask.get("parameters").unwrap();
        assert_eq!(
            ask_params
                .get("required")
                .and_then(Value::as_array)
                .unwrap(),
            &[Value::string("title"), Value::string("question")]
        );
        let names = |delegation| {
            crate::tools::definitions_for(delegation)
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                .collect::<Vec<_>>()
        };
        assert!(names(true).contains(&"ask_user"));
        assert!(!names(false).contains(&"ask_user"));
        assert!(names(false).contains(&"todo_list"));
    }
}
