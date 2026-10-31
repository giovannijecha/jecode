use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, tool_call},
    tools::Tools,
};

fn agent(directory: &Directory) -> Agent {
    let fixture = HttpFixture::new(vec![]);
    Agent::new(
        OpenRouter::fixture(fixture.endpoint),
        Tools::new(directory.path()).unwrap(),
    )
}

fn set_call(id: &str, title: &str, status: &str) -> crate::backend::ToolCall {
    let arguments = Value::object([
        ("action", Value::string("set")),
        ("title", Value::string(title)),
        (
            "tasks",
            Value::Array(vec![Value::object([
                ("id", Value::string("t1")),
                ("title", Value::string("Inspect")),
                ("status", Value::string(status)),
            ])]),
        ),
    ]);
    crate::backend::ToolCall {
        id: id.into(),
        name: "todo_list".into(),
        arguments: arguments.encode(),
    }
}

fn record(agent: &mut Agent, call: &crate::backend::ToolCall) -> Value {
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        (
            "tool_calls",
            Value::Array(vec![tool_call(
                &call.id,
                &call.name,
                crate::json::parse(&call.arguments).unwrap(),
            )]),
        ),
    ]));
    let result = agent.execute_tool(call);
    agent.tool_result(call, &result);
    result
}

#[test]
fn set_get_and_invalid_update_use_original_tool_history() {
    let directory = Directory::new();
    let mut agent = agent(&directory);
    let first = record(&mut agent, &set_call("one", "Export", "pending"));
    assert_eq!(first.get("revision").and_then(Value::as_usize), Some(1));
    let failed = record(&mut agent, &set_call("bad", "Export", "unknown"));
    assert!(failed.get("error").is_some());
    assert_eq!(agent.todo_plan().unwrap().revision, 1);
    let next = record(&mut agent, &set_call("two", "Export", "done"));
    assert_eq!(next.get("revision").and_then(Value::as_usize), Some(2));
    assert_eq!(agent.todo_plan().unwrap().tasks[0].status, "done");
    assert!(agent.request_environment().contains("revision 2"));
    let get = agent
        .todo_tool(&Value::object([("action", Value::string("get"))]))
        .unwrap();
    assert_eq!(get, next);
    let history = agent.messages.lock().unwrap();
    assert_eq!(
        crate::json::parse(history[2].get("content").unwrap().as_str().unwrap()).unwrap(),
        first
    );
}

#[test]
fn separate_agents_keep_separate_plans() {
    let parent_directory = Directory::new();
    let child_directory = Directory::new();
    let mut parent = agent(&parent_directory);
    let mut child = agent(&child_directory);
    child.child = true;
    record(&mut child, &set_call("child", "Inspect API", "done"));
    assert!(parent.todo_plan().is_none());
    record(&mut parent, &set_call("parent", "Ship feature", "pending"));
    assert_eq!(child.todo_plan().unwrap().title, "Inspect API");
    assert_eq!(parent.todo_plan().unwrap().title, "Ship feature");
}

#[test]
fn saved_plan_survives_resume_and_context_projection() {
    let directory = Directory::new();
    let home = Directory::new();
    let mut first = agent(&directory);
    first.enable_sessions(home.path()).unwrap();
    let id = first.sessions().unwrap().id();
    first.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Build the export flow")),
    ]));
    record(&mut first, &set_call("saved", "Build export", "pending"));
    record(&mut first, &set_call("updated", "Build export", "waiting"));
    first.context.from = first.messages.lock().unwrap().len();
    first.context.summary = "Earlier implementation compressed".into();
    first.save_session().unwrap();
    drop(first);
    let mut resumed = agent(&directory);
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    let plan = resumed.todo_plan().unwrap();
    assert_eq!(plan.revision, 2);
    assert_eq!(plan.tasks[0].status, "waiting");
    assert!(resumed.request_environment().contains("Build export"));
    let projected = resumed.transport_messages();
    assert!(
        projected
            .last()
            .unwrap()
            .get("content")
            .and_then(Value::as_str)
            .unwrap()
            .contains("revision 2")
    );
    assert!(
        !projected
            .iter()
            .any(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
    );
    let old_id = resumed.sessions().unwrap().id();
    resumed
        .start_new(resumed.model().into(), resumed.effort())
        .unwrap();
    assert!(resumed.todo_plan().is_none());
    resumed.resume(&old_id).unwrap();
    assert_eq!(resumed.todo_plan().unwrap(), plan);
}
