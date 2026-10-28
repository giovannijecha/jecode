use super::*;

#[test]
#[ignore = "requires a real interactive Windows console and manual keyboard input"]
fn interactive_windows_smoke() {
    let directory = Directory::new();
    fs::write(
        directory.path().join("fixture.txt"),
        (1..=80)
            .map(|n| format!("fixture line {n}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let fixture = HttpFixture::with_wait(
        vec![
            (
                200,
                completion(
                    "I will inspect the fixture.",
                    vec![
                        tool_call(
                            "read-1",
                            "read",
                            Value::object([("path", Value::string("fixture.txt"))]),
                        ),
                    ],
                ),
            ),
            (
                200,
                completion(
                    "\n  \n",
                    vec![
                        tool_call(
                            "write-1",
                            "write",
                            Value::object([
                                ("path", Value::string("generated.rs")),
                                ("content", Value::string("fn fixture() {\n    println!(\"before\");\n}\n")),
                            ]),
                        ),
                    ],
                ),
            ),
            (
                200,
                completion(
                    "",
                    vec![
                        tool_call(
                            "edit-1",
                            "edit",
                            Value::object([
                                ("path", Value::string("generated.rs")),
                                ("old_text", Value::string("before")),
                                ("new_text", Value::string("after")),
                            ]),
                        ),
                    ],
                ),
            ),
            (
                200,
                completion(
                    "",
                    vec![
                        tool_call(
                            "bash-1",
                            "bash",
                            Value::object([(
                                "command",
                                Value::string(
                                    "printf 'stdout fixture\\n'; printf 'stderr fixture\\n' >&2; sleep 2",
                                ),
                            )]),
                        ),
                    ],
                ),
            ),
            (
                200,
                completion(
                    "",
                    vec![
                        tool_call("error-1", "bash", Value::object([
                            ("command", Value::string("printf 'fixture command failed\\n' >&2; exit 7")),
                        ])),
                    ],
                ),
            ),
            (200, completion(
                "# Tool fixture completed\nThe tool output stays compact.\n\n```rust\nlet value = 1;\n\nprintln!(\"verified\");\n```\n\nSend a second task to inspect conversation scrolling.",
                vec![])),
            (200, completion(
                &format!("# Fixture verified\n\nAll four tools returned captured results.\nOne fixture command intentionally exited with **code 7**.\n\n- Inspect the compact tool previews above.\n- Scroll the conversation with the wheel or Page Up/Down.\n- Browse prompt history with Up/Down.\n\n```rust\nfn fixture() {{\n    let max_requests = 40;\n    println!(\"verified\"); // display fixture\n}}\n```\n\n{}",
                    "This later paragraph keeps the original tool cards in the conversation history.\n\n".repeat(30)),
                vec![])),
        ],
        Duration::from_secs(180),
    );
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    launch(agent, config(&directory), None, Default::default(), None).unwrap();
    assert_eq!(fixture.finish().len(), 7);
    let exports: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("JECODE-SESSION-")
        })
        .collect();
    assert!(!exports.is_empty(), "use /export during the smoke test");
    let text = fs::read_to_string(exports.last().unwrap().path()).unwrap();
    assert!(!text.contains("isolated-fixture-key"));
    assert!(json::parse(&text).is_ok());
    assert!(
        text.contains("fixture line 80"),
        "exports keep captured output beyond the preview"
    );
}

#[test]
#[ignore = "requires a real interactive Windows console and manual Ask/Todo input"]
fn work_tools_windows_smoke() {
    let directory = Directory::new();
    let fixture = HttpFixture::with_wait(
        vec![
            (
                200,
                completion(
                    "I will track this work and confirm the export scope.",
                    vec![tool_call(
                        "plan-1",
                        "todo_list",
                        Value::object([
                            ("action", Value::string("set")),
                            ("title", Value::string("CSV export")),
                            (
                                "tasks",
                                Value::Array(vec![Value::object([
                                    ("id", Value::string("t1")),
                                    ("title", Value::string("Choose exported rows")),
                                    ("status", Value::string("waiting")),
                                ])]),
                            ),
                        ]),
                    )],
                ),
            ),
            (
                200,
                completion(
                    "",
                    vec![tool_call(
                        "ask-1",
                        "ask_user",
                        Value::object([
                            ("title", Value::string("Export scope")),
                            (
                                "question",
                                Value::string("Which rows should the CSV include?"),
                            ),
                            (
                                "options",
                                Value::Array(vec![
                                    Value::object([
                                        ("id", Value::string("visible")),
                                        ("label", Value::string("Visible rows")),
                                        ("recommended", Value::Bool(true)),
                                    ]),
                                    Value::object([
                                        ("id", Value::string("all")),
                                        ("label", Value::string("All report rows")),
                                    ]),
                                ]),
                            ),
                        ]),
                    )],
                ),
            ),
            (
                200,
                completion(
                    "Answer recorded. Inspect /todo and the expanded receipts, then quit with Ctrl+Q.",
                    vec![],
                ),
            ),
        ],
        Duration::from_secs(180),
    );
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    // Send a prompt, leave Ask with Esc, inspect /todo, reopen with /ask,
    // then choose or type an answer and submit explicitly with Enter.
    launch(agent, config(&directory), None, Default::default(), None).unwrap();
    assert_eq!(fixture.finish().len(), 3);
}
