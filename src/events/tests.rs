use super::*;

#[test]
fn bash_summary_exposes_incomplete_checkpoint_status() {
    for status in ["partial", "none"] {
        let result = Value::object([
            ("exit_code", Value::number(0)),
            (
                "checkpoint",
                Value::object([("status", Value::string(status))]),
            ),
        ]);
        let summary = tool_summary("bash", &result);
        assert!(
            summary.contains(status),
            "{status} checkpoint was hidden from the tool card: {summary}"
        );
    }
}

#[test]
fn checkpoint_summaries_use_actual_preview_and_full_counts() {
    let restored = Value::object([
        ("ref", Value::string("rc-one")),
        ("preview", Value::Bool(false)),
        ("planned", Value::Array(vec![Value::object([])])),
        ("planned_count", Value::number(71)),
        ("restored", Value::Array(vec![Value::object([])])),
        ("restored_count", Value::number(60)),
        ("conflicts", Value::Array(vec![])),
        ("conflict_count", Value::number(0)),
        ("failed", Value::Array(vec![])),
        ("failed_count", Value::number(0)),
    ]);
    assert_eq!(
        tool_summary("checkpoint", &restored),
        "restored 60 paths from rc-one"
    );
    let mut preview = restored;
    if let Value::Object(fields) = &mut preview {
        fields.insert("preview".into(), Value::Bool(true));
    }
    assert_eq!(
        tool_summary("checkpoint", &preview),
        "preview: 71 paths would change"
    );
}

#[test]
fn checkpoint_summaries_distinguish_list_inspect_and_failures() {
    let listed = Value::object([("calls", Value::Array(vec![Value::object([])]))]);
    assert_eq!(tool_summary("checkpoint", &listed), "1 call");
    let inspected = Value::object([
        ("ref", Value::string("rc-one")),
        ("paths", Value::Array(vec![Value::object([])])),
        ("path_count", Value::number(80)),
    ]);
    assert_eq!(
        tool_summary("checkpoint", &inspected),
        "inspected 80 paths in rc-one"
    );
    let failed = Value::object([
        ("ref", Value::string("rc-one")),
        ("preview", Value::Bool(false)),
        ("restored_count", Value::number(1)),
        ("failed", Value::Array(vec![Value::object([])])),
        ("failed_count", Value::number(3)),
        ("conflict_count", Value::number(2)),
    ]);
    assert_eq!(
        tool_summary("checkpoint", &failed),
        "restored 1 path from rc-one; 2 conflicts; 3 paths failed"
    );
}

#[test]
fn local_web_and_browser_cards_show_actions_and_artifacts() {
    let fetch = Value::object([
        ("action", Value::string("fetch")),
        ("url", Value::string("https://example.org/")),
    ]);
    assert_eq!(description("web", &fetch), "fetch https://example.org/");
    let fetched = Value::object([("status", Value::number(200)), ("bytes", Value::number(42))]);
    assert_eq!(tool_summary("web", &fetched), "HTTP 200 · 42 bytes");

    let screenshot = Value::object([
        ("action", Value::string("screenshot")),
        ("tab_id", Value::string("tab-1")),
    ]);
    assert_eq!(description("browser", &screenshot), "screenshot tab-1");
    let captured = Value::object([("screenshot_path", Value::string("tmp:browser/shot.png"))]);
    assert_eq!(
        tool_summary("browser", &captured),
        "screenshot saved to tmp:browser/shot.png"
    );
    let crawl = Value::object([
        ("pages_in_call", Value::number(35)),
        ("pages_truncated", Value::Bool(true)),
        ("manifest_repaired", Value::Bool(true)),
        ("queued", Value::number(4)),
        (
            "pages",
            Value::Array(vec![Value::object([("error", Value::string("offline"))])]),
        ),
    ]);
    assert_eq!(
        tool_summary("web", &crawl),
        "crawled 35 pages; 1 failed in preview; 4 queued; manifest repaired"
    );
}

#[test]
fn several_read_paths_are_named_together() {
    let arguments = Value::object([(
        "paths",
        Value::Array(vec![Value::string("src/a.rs"), Value::string("README.md")]),
    )]);
    assert_eq!(description("read", &arguments), "src/a.rs, README.md");
}

#[test]
fn multi_file_edits_name_their_files_and_count_outcomes() {
    let file = |path: &str| Value::object([("path", Value::string(path))]);
    let arguments = Value::object([(
        "files",
        Value::Array(vec![file("src/a.rs"), file("src/b.rs"), file("README.md")]),
    )]);
    assert_eq!(
        description("edit", &arguments),
        "src/a.rs, src/b.rs, README.md"
    );
    let entry = |path: &str, outcome: (&str, Value)| {
        Value::object([("path", Value::string(path)), outcome])
    };
    let written = |path| entry(path, ("bytes_written", Value::number(12)));
    let failed = |path| {
        entry(
            path,
            ("error", Value::string("edit 1: old text was not found")),
        )
    };
    for (entries, expected) in [
        (vec![written("a"), written("b")], "changed 2 files"),
        (vec![written("a"), failed("b")], "changed 1 file; 1 failed"),
        (vec![failed("a"), failed("b")], "no files changed; 2 failed"),
    ] {
        let result = Value::object([("files", Value::Array(entries))]);
        assert_eq!(tool_summary("edit", &result), expected);
    }
}

fn member(path: &str, field: (&str, Value), checkpoint: Option<Value>) -> Value {
    let mut member = Value::object([("path", Value::string(path)), field]);
    if let (Value::Object(fields), Some(checkpoint)) = (&mut member, checkpoint) {
        fields.insert("checkpoint".into(), checkpoint);
    }
    member
}

fn batch(members: Vec<Value>) -> Value {
    Value::object([("files", Value::Array(members))])
}

#[test]
fn multi_file_edits_warn_about_each_members_checkpoint() {
    let receipt = |status: &str, reason: Option<&str>| match reason {
        Some(reason) => Value::object([
            ("status", Value::string(status)),
            ("reason", Value::string(reason)),
        ]),
        None => Value::object([("status", Value::string(status))]),
    };
    let written = |path, checkpoint| member(path, ("bytes_written", Value::number(4)), checkpoint);
    let full = || receipt("none", Some("disk full"));
    for (members, expected) in [
        (
            vec![
                written("a.rs", Some(receipt("saved", None))),
                written("b.rs", None),
            ],
            "changed 2 files",
        ),
        (
            vec![
                written("a.rs", Some(receipt("saved", None))),
                written("b.rs", Some(receipt("partial", None))),
            ],
            "changed 2 files; checkpoint partial (b.rs)",
        ),
        (
            vec![
                written("a.rs", Some(full())),
                written("b.rs", Some(receipt("saved", None))),
                written("c.rs", Some(full())),
            ],
            "changed 3 files; checkpoint none: disk full (a.rs, c.rs)",
        ),
        (
            vec![
                written("a.rs", Some(full())),
                written("b.rs", Some(receipt("partial", Some("locked")))),
                member("c.rs", ("error", Value::string("no match")), None),
            ],
            "changed 2 files; 1 failed; checkpoint none: disk full (a.rs); checkpoint partial: locked (b.rs)",
        ),
    ] {
        assert_eq!(tool_summary("edit", &batch(members)), expected);
    }
    // One file keeps its own wording.
    let single = Value::object([("bytes_written", Value::number(4)), ("checkpoint", full())]);
    assert_eq!(
        tool_summary("edit", &single),
        "wrote 4 bytes; checkpoint none: disk full"
    );
}

#[test]
fn multi_path_reads_count_the_files_they_could_not_read() {
    let read = |path| member(path, ("content", Value::string("1: a\n")), None);
    let missed = |path| member(path, ("error", Value::string("not found")), None);
    for (members, expected) in [
        (vec![read("a.rs"), read("b.rs")], "read 2 files"),
        (
            vec![read("a.rs"), missed("b.rs")],
            "read 1 file; 1 not read",
        ),
        (
            vec![missed("a.rs"), missed("b.rs")],
            "no files read; 2 not read",
        ),
    ] {
        assert_eq!(tool_summary("read", &batch(members)), expected);
    }
}

#[test]
fn a_call_that_never_ran_says_why_instead_of_how_it_finished() {
    for (outcome, says) in [
        ("denied", "Denied · nothing ran; the agent was told"),
        ("unavailable", "Nothing ran"),
    ] {
        let result = Value::object([
            ("outcome", Value::string(outcome)),
            ("reason", Value::string("Nothing ran")),
        ]);
        for name in ["edit", "read", "write", "bash", "exec", "list", "web"] {
            assert_eq!(tool_summary(name, &result), says, "{name}");
        }
    }
    // Each still reports through its own route.
    let unanswered = Value::object([("outcome", Value::string("unavailable"))]);
    assert_eq!(
        tool_summary("ask_user", &unanswered),
        "Interactive input unavailable"
    );
    let interrupted = Value::object([
        ("outcome", Value::string("not_executed")),
        ("error", Value::string("Tool was not executed")),
    ]);
    assert_eq!(
        tool_summary("edit", &interrupted),
        "error: Tool was not executed"
    );
}

#[test]
fn an_emptied_file_counts_as_changed_and_a_no_op_does_not() {
    use crate::{cancel::Cancellation, checkpoints::session::Session, tools::Begun};
    let root = crate::test_support::Directory::new();
    let home = crate::test_support::Directory::new();
    std::fs::write(root.path().join("emptied.rs"), "all\n").unwrap();
    std::fs::write(root.path().join("same.rs"), "same\n").unwrap();
    let mut tools = crate::tools::Tools::new(root.path()).unwrap();
    tools.configure_checkpoints(Session::open(home.path(), tools.root()));
    let file = |path: &str, old: &str, new: &str| {
        Value::object([
            ("path", Value::string(path)),
            (
                "edits",
                Value::Array(vec![Value::object([
                    ("old_text", Value::string(old)),
                    ("new_text", Value::string(new)),
                ])]),
            ),
        ])
    };
    let run = |files: Vec<Value>| {
        let arguments = Value::object([("files", Value::Array(files))]).encode();
        tools
            .execute_begun(
                Begun::default(),
                "edit",
                &arguments,
                &Cancellation::default(),
            )
            .result
    };
    let result = run(vec![
        file("emptied.rs", "all\n", ""),
        file("same.rs", "same", "same"),
    ]);
    assert_eq!(
        std::fs::read_to_string(root.path().join("emptied.rs")).unwrap(),
        ""
    );
    assert_eq!(
        tool_summary("edit", &result),
        "changed 1 file",
        "{result:?}"
    );
    let result = run(vec![file("same.rs", "same", "same")]);
    assert_eq!(tool_summary("edit", &result), "no files changed");
}

#[test]
fn routine_description_overrides_command_and_blank_falls_back() {
    for (name, key, command, fallback) in [
        (
            "read",
            "path",
            "src/lib.rs",
            "src/lib.rs (from line 1, up to 200 lines)",
        ),
        ("bash", "command", "cargo test", "cargo test"),
        ("exec", "executable", "cargo", "cargo"),
        ("process", "action", "list", "list"),
        ("web", "action", "fetch", "fetch"),
        ("browser", "action", "tabs", "tabs"),
    ] {
        let arguments = Value::object([
            (key, Value::string(command)),
            ("description", Value::string("Inspect relevant results")),
        ]);
        assert_eq!(
            description(name, &arguments),
            "Inspect relevant results",
            "{name}"
        );
        assert_eq!(target(name, &arguments), fallback, "{name}");
        for blank in ["", " \t "] {
            let arguments = Value::object([
                (key, Value::string(command)),
                ("description", Value::string(blank)),
            ]);
            assert_eq!(description(name, &arguments), fallback, "{name}");
        }
        let arguments = Value::object([(key, Value::string(command))]);
        assert_eq!(description(name, &arguments), fallback, "{name}");
    }
}

#[test]
fn agent_target_uses_shown_number_and_keeps_unknown_ids_honest() {
    let shown = Value::object([
        ("action", Value::string("wait")),
        ("id", Value::string("a3")),
        ("display_number", Value::number(1)),
    ]);
    assert_eq!(target("agent", &shown), "wait AGENT 1");
    let unknown = Value::object([
        ("action", Value::string("inspect")),
        ("id", Value::string("a3")),
    ]);
    assert_eq!(target("agent", &unknown), "inspect a3");
}

#[test]
fn exec_summary_uses_command_exit_and_background_contract() {
    let done = Value::object([
        ("exit_code", Value::number(3)),
        ("truncated", Value::Bool(true)),
        ("stdout_file", Value::string("output:fixture:stdout")),
    ]);
    assert_eq!(tool_summary("exec", &done), "exit 3; full output saved");
    let background = Value::object([
        ("process_id", Value::string("process-fixture")),
        ("status", Value::string("running")),
    ]);
    assert_eq!(
        tool_summary("exec", &background),
        "started in background · process-fixture"
    );
    let arguments = Value::object([
        ("executable", Value::string("cargo")),
        (
            "args",
            Value::Array(vec![Value::string("test"), Value::string("a b")]),
        ),
    ]);
    assert_eq!(target("exec", &arguments), "cargo [\"test\",\"a b\"]");
}
