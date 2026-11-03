use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::test_support::Directory;
use crate::tools::Tools;

fn read(agent: &Agent, arguments: Value) -> Value {
    agent.execute_tool(&ToolCall {
        id: "batch".into(),
        name: "read".into(),
        arguments: arguments.encode(),
    })
}

fn paths(paths: &[&str]) -> Value {
    Value::object([(
        "paths",
        Value::Array(paths.iter().map(|path| Value::string(*path)).collect()),
    )])
}

#[test]
fn several_files_are_read_in_order_and_each_reports_its_own_outcome() {
    let directory = Directory::new();
    let outside = Directory::new();
    std::fs::write(directory.path().join("a.txt"), "alpha\n").unwrap();
    std::fs::write(directory.path().join("b.txt"), "beta\n").unwrap();
    std::fs::write(outside.path().join("secret.txt"), "hidden\n").unwrap();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(directory.path()).unwrap(),
    );
    let secret = outside.path().join("secret.txt");
    let result = read(
        &agent,
        paths(&["a.txt", "missing.txt", "b.txt", &secret.to_string_lossy()]),
    );
    let files = result.get("files").and_then(Value::as_array).unwrap();
    let field = |index: usize, key: &str| {
        files[index]
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(files.len(), 4);
    assert_eq!(field(0, "path"), "a.txt");
    assert!(field(0, "content").contains("alpha"));
    assert!(!field(1, "error").is_empty());
    assert!(field(2, "content").contains("beta"));
    // Outside paths keep their single-read approval flow.
    assert!(field(3, "error").contains("read it alone with path"));
    assert!(!result.encode().contains("hidden"));
}

#[test]
fn invalid_batches_fail_before_reading() {
    let directory = Directory::new();
    std::fs::write(directory.path().join("a.txt"), "alpha\n").unwrap();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut both = paths(&["a.txt"]);
    if let Value::Object(fields) = &mut both {
        fields.insert("path".into(), Value::string("a.txt"));
    }
    assert!(read(&agent, both).get("error").is_some());
    assert!(read(&agent, paths(&[])).get("error").is_some());
    let many = vec!["a.txt"; MAX_PATHS + 1];
    assert!(read(&agent, paths(&many)).get("error").is_some());
}
