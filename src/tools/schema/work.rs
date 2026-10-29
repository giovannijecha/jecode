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
                    "Optional collaborator ID, such as a1, that works on this task. Its return shows the task as awaiting your review and its failure as failed until you mark the task; a returned report never completes it.",
                ),
            ),
        ]),
        &["id", "title", "status"],
    );
    super::tool(
        "todo_list",
        "Own the current conversation's plan; the user reads it in /todo. get reads it; set atomically replaces the full task list and records a chronological snapshot. Keep existing task IDs when changing title, status or note; use cancelled for work removed by a decision. Only the main agent marks its plan and decides when a task is done, after reviewing any collaborator's returned work. Collaborators keep their own plans and never complete the main agent's tasks. Calls run in order even with parallel:true.",
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
    let question = object(
        Value::object([
            ("id", string("Stable question ID in this call.")),
            ("title", string("Short title shown in the question steps.")),
            (
                "question",
                string("One question requiring the user's decision or missing information."),
            ),
            (
                "context",
                string("Optional concise context for the decision."),
            ),
            (
                "multiple",
                Value::object([
                    ("type", Value::string("boolean")),
                    (
                        "description",
                        Value::string("True when the user may choose several options."),
                    ),
                ]),
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
        &["id", "title", "question"],
    );
    super::tool(
        "ask_user",
        "Ask the user one to four focused questions and wait for explicit answers; they are answered one at a time and sent together. Each question may offer single or multiple choice options, and the user can always answer in their own words. Nothing is preselected and a recommendation is not an answer. The result lists each question's chosen option IDs and typed text. If interactive input is unavailable or the questions are dismissed, report that no answer was supplied; do not infer consent. Collaborators cannot ask the user directly. Calls run in order even with parallel:true.",
        Value::object([(
            "questions",
            Value::object([
                ("type", Value::string("array")),
                ("items", question),
                ("minItems", Value::number(1)),
                ("maxItems", Value::number(4)),
            ]),
        )]),
        &["questions"],
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
            &[Value::string("questions")]
        );
        let questions = ask_params
            .get("properties")
            .and_then(|properties| properties.get("questions"))
            .unwrap();
        assert_eq!(
            questions.get("maxItems").map(Value::encode),
            Some("4".into())
        );
        assert_eq!(
            questions
                .get("items")
                .and_then(|item| item.get("required"))
                .map(Value::encode),
            Some(r#"["id","title","question"]"#.into())
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
