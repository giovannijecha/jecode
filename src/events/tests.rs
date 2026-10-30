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
