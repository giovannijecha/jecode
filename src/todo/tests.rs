use super::*;

fn set_call(id: &str) -> Value {
    call(id, "todo_list", "set")
}

fn call(id: &str, name: &str, action: &str) -> Value {
    Value::object([
        ("role", Value::string("assistant")),
        (
            "tool_calls",
            Value::Array(vec![
                crate::conversation::ToolCall {
                    id: id.into(),
                    name: name.into(),
                    arguments: Value::object([("action", Value::string(action))]).encode(),
                }
                .value(),
            ]),
        ),
    ])
}

#[test]
fn reused_call_ids_only_match_their_own_assistant_batch() {
    let first = Plan::set(&args("Export", "pending"), None).unwrap();
    let next = Plan::set(&args("Export", "done"), Some(&first)).unwrap();
    let mut history = vec![
        set_call("reused"),
        result("reused", first.value()),
        call("reused", "todo_list", "get"),
        result("reused", next.value()),
        call("reused", "read", "get"),
        result("reused", next.value()),
    ];
    assert_eq!(latest(&history), Some(first));
    history.extend([set_call("new"), result("new", next.value())]);
    assert_eq!(latest(&history), Some(next));
}

fn result(id: &str, content: Value) -> Value {
    Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string(id)),
        ("content", Value::string(content.encode())),
    ])
}

fn args(title: &str, status: &str) -> Value {
    Value::object([
        ("title", Value::string(title)),
        (
            "tasks",
            Value::Array(vec![Value::object([
                ("id", Value::string("t1")),
                ("title", Value::string("Review work")),
                ("status", Value::string(status)),
            ])]),
        ),
    ])
}

#[test]
fn newest_successful_snapshot_wins_without_rewriting_earlier_result() {
    let first = Plan::set(&args("Export", "pending"), None).unwrap();
    let next = Plan::set(&args("Export", "done"), Some(&first)).unwrap();
    let history = vec![
        set_call("one"),
        result("one", first.value()),
        set_call("bad"),
        result(
            "bad",
            Value::object([("error", Value::string("bad status"))]),
        ),
        set_call("two"),
        result("two", next.value()),
    ];
    assert_eq!(latest(&history), Some(next));
    assert_eq!(
        Plan::parse(&json::parse(history[1].get("content").unwrap().as_str().unwrap()).unwrap())
            .unwrap(),
        first
    );
}

#[test]
fn invalid_set_rejects_duplicate_ids_and_unknown_status() {
    let task = Value::object([
        ("id", Value::string("t1")),
        ("title", Value::string("Review")),
        ("status", Value::string("pending")),
    ]);
    let duplicate = Value::object([
        ("title", Value::string("Work")),
        ("tasks", Value::Array(vec![task.clone(), task])),
    ]);
    assert!(
        Plan::set(&duplicate, None)
            .unwrap_err()
            .contains("Duplicate task id")
    );
    assert!(
        Plan::set(&args("Work", "started"), None)
            .unwrap_err()
            .contains("Invalid task status")
    );
}

#[test]
fn get_result_and_other_tool_results_do_not_create_a_plan() {
    let history = vec![
        Value::object([
            ("role", Value::string("assistant")),
            (
                "tool_calls",
                Value::Array(vec![
                    crate::conversation::ToolCall {
                        id: "get".into(),
                        name: "todo_list".into(),
                        arguments: Value::object([("action", Value::string("get"))]).encode(),
                    }
                    .value(),
                ]),
            ),
        ]),
        result("get", Plan::empty().value()),
        result(
            "unrelated",
            Plan::set(&args("False", "done"), None).unwrap().value(),
        ),
    ];
    assert_eq!(latest(&history), None);
    assert_eq!(Plan::parse(&Plan::empty().value()).unwrap(), Plan::empty());
}
