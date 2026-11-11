use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::test_support::{Directory, HttpFixture, completion, tool_call};
use crate::tools::Tools;
use std::fs;

mod failure;

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
fn a_skill_read_through_paths_stays_tracked_after_a_rebuild() {
    let project = Directory::new();
    let home = Directory::new();
    let package = home.path().join("skills/audit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("SKILL.md"),
        "---\nname: audit\ndescription: Audit a change.\n---\n\n# Audit\n",
    )
    .unwrap();
    std::fs::write(project.path().join("a.txt"), "alpha\n").unwrap();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.prepare_turn("Inspect").unwrap();
    assert!(agent.skill_checkpoint_instructions().is_empty());
    let call = ToolCall {
        id: "batch".into(),
        name: "read".into(),
        arguments: paths(&["skill:audit", "a.txt"]).encode(),
    };
    let result = agent.execute_tool(&call);
    agent.tool_result(&call, &result);
    assert!(
        agent
            .skill_checkpoint_instructions()
            .contains("skill:audit"),
        "{}",
        result.encode()
    );
    // Compaction and resume rebuild the list from the saved receipts.
    agent.used_skills.clear();
    agent.rebuild_used_skills();
    assert!(
        agent
            .skill_checkpoint_instructions()
            .contains("skill:audit"),
        "{}",
        result.encode()
    );
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
}

#[test]
fn a_skill_saved_after_the_inline_results_stays_tracked_after_a_rebuild() {
    let project = Directory::new();
    let home = Directory::new();
    let package = home.path().join("skills/audit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("SKILL.md"),
        "---\nname: audit\ndescription: Audit a change.\n---\n\n# Audit\n",
    )
    .unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(
            project.path().join(name),
            format!("{}\n", "x".repeat(50_000)),
        )
        .unwrap();
    }
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.prepare_turn("Inspect").unwrap();
    let call = ToolCall {
        id: "batch".into(),
        name: "read".into(),
        arguments: paths(&["a.txt", "b.txt", "c.txt", "skill:audit"]).encode(),
    };
    let result = agent.execute_tool(&call);
    assert!(result.get("saved_files").and_then(Value::as_usize) >= Some(1));
    assert!(
        result
            .get("files")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .all(|file| file.get("skill_reference").is_none())
    );
    agent.tool_result(&call, &result);
    assert!(
        agent
            .skill_checkpoint_instructions()
            .contains("skill:audit")
    );
    agent.used_skills.clear();
    agent.rebuild_used_skills();
    assert!(
        agent
            .skill_checkpoint_instructions()
            .contains("skill:audit")
    );
}

#[test]
fn a_large_batch_reads_every_path_and_saves_what_exceeds_the_inline_size() {
    let directory = Directory::new();
    // 60 files of about 4 KiB each: past the former 12-path cap and well past
    // the inline size, so most results go to the result file.
    let names: Vec<String> = (0..60).map(|index| format!("f{index:02}.txt")).collect();
    for (index, name) in names.iter().enumerate() {
        let body = format!("marker-{index:02}\n{}\n", "x".repeat(4000));
        fs::write(directory.path().join(name), body).unwrap();
    }
    let mut requested: Vec<&str> = names.iter().map(String::as_str).collect();
    requested.insert(30, "missing.txt");
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(directory.path()).unwrap(),
    );
    let result = read(&agent, paths(&requested));
    assert!(result.get("error").is_none(), "{}", result.encode());
    let inline = result.get("files").and_then(Value::as_array).unwrap();
    assert!(!inline.is_empty() && inline.len() < requested.len());
    assert!(result.encode().len() < 100 * 1024);
    let saved = result.get("saved_files").and_then(Value::as_usize).unwrap();
    assert_eq!(inline.len() + saved, requested.len());
    assert_eq!(result.get("files_read"), Some(&Value::number(60)));
    assert_eq!(result.get("files_not_read"), Some(&Value::number(1)));
    assert_eq!(result.get("lines_read"), Some(&Value::number(120)));

    // The saved entries are the rest of the call, in order, through the
    // ordinary read of an output reference.
    let reference = result.get("result_file").and_then(Value::as_str).unwrap();
    let mut text = String::new();
    let mut offset = 1;
    loop {
        let page = agent.execute_tool(&ToolCall {
            id: "page".into(),
            name: "read".into(),
            arguments: Value::object([
                ("path", Value::string(reference)),
                ("offset", Value::number(offset)),
                ("limit", Value::number(2000)),
            ])
            .encode(),
        });
        text.push_str(page.get("content").and_then(Value::as_str).unwrap());
        match page.get("next_offset").and_then(Value::as_usize) {
            Some(next) => offset = next,
            None => break,
        }
    }
    let path_of = |entry: &Value| {
        entry
            .get("path")
            .and_then(Value::as_str)
            .unwrap()
            .to_owned()
    };
    let mut order: Vec<String> = inline.iter().map(path_of).collect();
    order.extend(
        text.lines()
            .filter_map(|line| line.split_once("\"path\": \"").map(|(_, rest)| rest))
            .map(|rest| rest.trim_end_matches(['"', ',']).to_owned()),
    );
    assert_eq!(order, requested);
    assert!(text.contains("marker-59"));
    assert!(
        text.contains("\"error\""),
        "the missing file keeps its own error"
    );
    assert_eq!(
        crate::events::tool_summary("read", &result),
        format!("read 60 files; 1 not read; {saved} saved to result_file")
    );
}

#[test]
fn a_multi_path_read_reads_its_files_beside_other_calls() {
    let project = Directory::new();
    fs::write(project.path().join("a.txt"), "alpha\n").unwrap();
    fs::write(project.path().join("b.txt"), "beta\n").unwrap();
    let paths = || Value::Array(vec![Value::string("a.txt"), Value::string("b.txt")]);
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    tool_call("batch", "read", Value::object([("paths", paths())])),
                    tool_call(
                        "single",
                        "read",
                        Value::object([("path", Value::string("a.txt"))]),
                    ),
                    tool_call(
                        "declared",
                        "read",
                        Value::object([("paths", paths()), ("parallel", Value::Bool(true))]),
                    ),
                    tool_call(
                        "neighbour",
                        "read",
                        Value::object([
                            ("path", Value::string("b.txt")),
                            ("parallel", Value::Bool(true)),
                        ]),
                    ),
                ],
            ),
        ),
        (200, completion("Done", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.run_turn("Read them", &mut |_| Ok(())).unwrap();
    let messages = agent.archive().messages.lock().unwrap().clone();
    for id in ["batch", "declared"] {
        let content = messages
            .iter()
            .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .unwrap();
        let result = crate::json::parse(content).unwrap();
        let files = result.get("files").and_then(Value::as_array).unwrap();
        assert_eq!(files.len(), 2, "{id}: {content}");
        assert!(
            content.contains("alpha") && content.contains("beta"),
            "{id}: {content}"
        );
    }
    assert_eq!(fixture.finish().len(), 2);
}
