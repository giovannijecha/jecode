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
fn only_media_views_wait_for_a_single_read() {
    let directory = Directory::new();
    let home = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let png = crate::attachments::tests::png(4, 3);
    std::fs::write(directory.path().join("shot.png"), &png).unwrap();
    let pool = agent.sessions().unwrap().store().attachments();
    let attached = |name: &str, bytes: &[u8]| pool.import_bytes(name, bytes).unwrap().reference();
    let text = attached("notes.txt", b"alpha\n");
    let binary = attached("blob.bin", &[0, 1, 2, 3]);
    let image = attached("shot.png", &png);
    let result = read(&agent, paths(&[&text, &binary, &image, "shot.png"]));
    let files = result.get("files").and_then(Value::as_array).unwrap();
    let field = |index: usize, key: &str| {
        files[index]
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    // Text pages arrive in this call, as the schema says.
    assert!(
        field(0, "content").contains("alpha"),
        "{}",
        files[0].encode()
    );
    assert_eq!(field(0, "error"), "");
    // Other files return a path, which needs no second read either.
    assert!(!field(1, "local_path").is_empty(), "{}", files[1].encode());
    assert_eq!(field(1, "error"), "");
    // Only a view has to wait, and it counts as not read.
    for index in [2, 3] {
        assert!(
            field(index, "error").contains("read it alone with path"),
            "{}",
            files[index].encode()
        );
        assert!(files[index].get("view").is_none());
    }
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
