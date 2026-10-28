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

#[test]
fn only_a_set_without_an_open_plan_creates_one() {
    let set = |title| {
        let mut arguments = args(title, "pending");
        if let Value::Object(fields) = &mut arguments {
            fields.insert("action".into(), Value::string("set"));
        }
        arguments
    };
    let get = Value::object([("action", Value::string("get"))]);
    let open = Plan::set(&args("Export", "in_progress"), None).unwrap();
    let ended = Plan::set(&args("Export", "done"), Some(&open)).unwrap();
    let dropped = Plan::set(&args("Export", "cancelled"), Some(&open)).unwrap();
    assert!(creates(None, &set("Export")));
    assert!(!creates(None, &get));
    assert!(!creates(Some(&open), &set("Replan")));
    assert!(!creates(Some(&ended), &get));
    assert!(creates(Some(&ended), &set("Next")));
    assert!(creates(Some(&dropped), &set("Next")));
    assert!(creates(Some(&Plan::empty()), &set("Next")));
}

fn task(status: &str, agent: Option<&str>) -> Task {
    Task {
        id: "t1".into(),
        title: "Review work".into(),
        status: status.into(),
        note: String::new(),
        agent: agent.map(Into::into),
    }
}

fn record(id: &str, number: usize, state: &str) -> Value {
    Value::object([
        ("id", Value::string(id)),
        ("number", Value::number(number)),
        ("state", Value::string(state)),
    ])
}

#[test]
fn collaborator_lifecycle_shows_until_the_main_agent_marks_the_task() {
    let records = [
        record("a1", 1, "returned"),
        record("a2", 2, "failed"),
        record("a3", 3, "running"),
    ];
    let phase = |status, agent| {
        let task = task(status, agent);
        Phase::of(&task, owner(&task, &records))
    };
    assert_eq!(phase("pending", None), Phase::NotStarted);
    assert_eq!(phase("in_progress", None), Phase::InProgress);
    assert_eq!(phase("waiting", Some("a1")), Phase::Waiting);
    assert_eq!(phase("in_progress", Some("a1")), Phase::AwaitingReview);
    assert_eq!(phase("pending", Some("AGENT 2")), Phase::Failed);
    assert_eq!(phase("in_progress", Some("a3")), Phase::InProgress);
    assert_eq!(phase("done", Some("a2")), Phase::Done);
    assert_eq!(phase("cancelled", Some("a1")), Phase::Cancelled);
    assert_eq!(phase("in_progress", Some("a9")), Phase::InProgress);
    assert_eq!(Phase::AwaitingReview.word(false), "awaiting review");
    assert_eq!(Phase::AwaitingReview.word(true), "review");
    assert_eq!(Phase::NotStarted.word(false), "");
}

#[test]
fn owners_resolve_by_id_or_number_and_label_in_capitals() {
    let records = [record("a2", 2, "running")];
    let label = |agent| {
        let task = task("pending", agent);
        owner_label(&task, owner(&task, &records))
    };
    assert_eq!(label(None), None);
    assert_eq!(label(Some("  ")), None);
    assert_eq!(label(Some("a2")), Some("AGENT 2".into()));
    assert_eq!(label(Some("agent-2")), Some("AGENT 2".into()));
    assert_eq!(label(Some("7")), Some("AGENT 7".into()));
    assert_eq!(label(Some("reviewer")), Some("REVIEWER".into()));
    let task = task("pending", Some("2"));
    assert_eq!(owner(&task, &records), Some(&records[0]));
}
