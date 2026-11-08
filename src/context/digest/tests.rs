use super::*;
use crate::json::Value;
use std::collections::BTreeSet;

mod outcomes;

fn user(text: &str) -> Value {
    Value::object([
        ("role", Value::string("user")),
        ("content", Value::string(text)),
    ])
}

fn assistant(text: &str, calls: &[(&str, &str, Value)]) -> Value {
    Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string(text)),
        (
            "tool_calls",
            Value::Array(
                calls
                    .iter()
                    .map(|(id, name, arguments)| {
                        Value::object([
                            ("id", Value::string(*id)),
                            ("name", Value::string(*name)),
                            ("arguments", Value::string(arguments.encode())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn tool(id: &str, result: Value) -> Value {
    Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string(id)),
        ("content", Value::string(result.encode())),
    ])
}

fn history() -> Vec<Value> {
    vec![
        Value::object([
            ("role", Value::string("system")),
            ("content", Value::string("system")),
        ]),
        user("Fix the login query. Every change gets a CHANGELOG line."),
        assistant(
            "",
            &[(
                "1",
                "read",
                Value::object([("path", Value::string("src/tokenizer.js"))]),
            )],
        ),
        tool(
            "1",
            Value::object([
                ("content", Value::string("1: code\n".repeat(200))),
                ("lines_returned", Value::number(200)),
                ("total_lines", Value::Null),
                ("offset", Value::number(1)),
            ]),
        ),
        assistant(
            "The tokenizer splits at the space.",
            &[(
                "2",
                "edit",
                Value::object([
                    ("path", Value::string("src/tokenizer.js")),
                    ("old_text", Value::string("a\nb")),
                    ("new_text", Value::string("a\nb\nc")),
                ]),
            )],
        ),
        tool("2", Value::object([("changed", Value::Bool(true))])),
        assistant(
            "",
            &[(
                "3",
                "bash",
                Value::object([("command", Value::string("node --test"))]),
            )],
        ),
        tool(
            "3",
            Value::object([
                ("exit_code", Value::number(1)),
                ("stdout", Value::string("noise\nnot ok 3 - parses quotes\n")),
                ("stderr", Value::string("")),
            ]),
        ),
        assistant("Fixed. Tests: 66/66", &[]),
    ]
}

fn epoch_text(level: u8) -> String {
    let messages = history();
    Epoch::new(1, messages.len(), level, String::new())
        .text(&messages, &BTreeSet::new())
        .to_owned()
}

#[test]
fn records_keep_requests_answers_changes_and_failures_exactly() {
    let text = epoch_text(1);
    assert!(text.contains("[1] User: Fix the login query. Every change gets a CHANGELOG line."));
    assert!(text.contains("[3] read src/tokenizer.js (lines 1-200, more remain)"));
    assert!(text.contains("[4] You: The tokenizer splits at the space."));
    assert!(text.contains("[5] edit src/tokenizer.js (+3 -2): c"));
    assert!(text.contains("[7] bash node --test → exit 1"));
    assert!(text.contains("not ok 3 - parses quotes"));
    assert!(text.contains("[8] You: Fixed. Tests: 66/66"));
    assert!(!text.contains("1: code"), "payloads stay in history");
}

#[test]
fn records_stay_byte_stable_after_saving_and_as_history_grows() {
    let mut messages = history();
    let end = messages.len();
    for level in [1, 2, 3] {
        let epoch = Epoch::new(1, end, level, "note".into());
        let before = epoch.text(&messages, &BTreeSet::new()).to_owned();
        let saved = Epoch::parse(&epoch.value()).unwrap();
        messages.push(user("A later request"));
        messages.push(assistant("A later answer", &[]));
        assert_eq!(before, saved.text(&messages, &BTreeSet::new()));
    }
}

#[test]
fn refused_and_skipped_calls_never_read_as_done() {
    let messages = vec![
        user("go"),
        assistant(
            "",
            &[(
                "1",
                "edit",
                Value::object([
                    ("path", Value::string("src/a.rs")),
                    ("old_text", Value::string("old")),
                    ("new_text", Value::string("new")),
                ]),
            )],
        ),
        tool("1", Value::object([("outcome", Value::string("denied"))])),
        assistant(
            "",
            &[(
                "2",
                "steps",
                Value::object([(
                    "steps",
                    Value::Array(vec![Value::object([
                        ("tool", Value::string("search")),
                        (
                            "arguments",
                            Value::object([("query", Value::string("needle"))]),
                        ),
                    ])]),
                )]),
            )],
        ),
        tool(
            "2",
            Value::object([(
                "steps",
                Value::Array(vec![Value::object([
                    ("tool", Value::string("search")),
                    ("outcome", Value::string("not_run")),
                ])]),
            )]),
        ),
    ];
    let detailed = Epoch::new(0, messages.len(), 1, String::new())
        .text(&messages, &BTreeSet::new())
        .to_owned();
    assert!(detailed.contains("edit src/a.rs → denied"), "{detailed}");
    assert!(detailed.contains("→ not_run"), "{detailed}");
    assert!(!detailed.contains("0 matches"), "{detailed}");
    let condensed = Epoch::new(0, messages.len(), 2, String::new())
        .text(&messages, &BTreeSet::new())
        .to_owned();
    assert!(!condensed.contains("Changed:"), "{condensed}");
    assert!(condensed.contains("Failed calls: 2"), "{condensed}");
}

#[test]
fn a_users_answer_survives_every_level_of_condensing() {
    let long_answer = format!(
        "Use Postgres {} KEEP-THE-AUDIT-TABLE",
        "because ".repeat(30)
    );
    let messages = vec![
        user("go"),
        assistant(
            "",
            &[(
                "1",
                "ask_user",
                Value::object([(
                    "questions",
                    Value::Array(vec![Value::object([(
                        "question",
                        Value::string("Which store?"),
                    )])]),
                )]),
            )],
        ),
        tool(
            "1",
            Value::object([
                ("outcome", Value::string("answered")),
                (
                    "answers",
                    Value::Array(vec![Value::object([
                        ("id", Value::string("store")),
                        ("choices", Value::Array(vec![])),
                        ("text", Value::string(&long_answer)),
                    ])]),
                ),
            ]),
        ),
    ];
    for level in [1, 2, 3] {
        let text = Epoch::new(0, messages.len(), level, String::new())
            .text(&messages, &BTreeSet::new())
            .to_owned();
        assert!(text.contains("Which store?"), "level {level}: {text}");
        assert!(text.contains("Use Postgres"), "level {level}: {text}");
        assert!(!text.contains("Failed calls"), "level {level}");
        // Beyond the 180 bytes other results keep.
        assert!(
            text.contains("KEEP-THE-AUDIT-TABLE"),
            "level {level}: {text}"
        );
    }
}

#[test]
fn condensed_levels_keep_the_request_answer_and_changed_files() {
    for level in [2, 3] {
        let text = epoch_text(level);
        assert!(text.contains("Every change gets a CHANGELOG line"));
        assert!(text.contains("Fixed. Tests: 66/66"));
        assert!(text.contains("Changed: src/tokenizer.js"));
        assert!(text.contains("Failed calls: 1"));
        assert!(!text.contains("read src/tokenizer.js"));
    }
    assert!(epoch_text(2).contains("Last check: bash node --test"));
    assert!(!epoch_text(3).contains("Last check"));
}

#[test]
fn steps_expand_into_one_record_per_step() {
    let messages = vec![
        user("go"),
        assistant(
            "",
            &[(
                "1",
                "steps",
                Value::object([(
                    "steps",
                    Value::Array(vec![Value::object([
                        ("tool", Value::string("write")),
                        (
                            "arguments",
                            Value::object([("path", Value::string("a.txt"))]),
                        ),
                    ])]),
                )]),
            )],
        ),
        tool(
            "1",
            Value::object([(
                "steps",
                Value::Array(vec![Value::object([
                    ("tool", Value::string("write")),
                    (
                        "result",
                        Value::object([("bytes_written", Value::number(7))]),
                    ),
                ])]),
            )]),
        ),
    ];
    let text = render(
        &messages,
        &Epoch::new(0, 3, 1, String::new()),
        &BTreeSet::new(),
    );
    assert!(text.contains("[2] write a.txt (7 bytes)"), "{text}");
}

#[test]
fn saved_epochs_round_trip_without_their_text() {
    let messages = history();
    let epoch = Epoch::new(1, 5, 2, "note".into());
    epoch.text(&messages, &BTreeSet::new());
    let saved = Value::Array(vec![epoch.value()]);
    assert!(!saved.encode().contains("tokenizer"));
    let parsed = parse(Some(&saved)).unwrap();
    assert_eq!(
        parsed[0].text(&messages, &BTreeSet::new()),
        epoch.text(&messages, &BTreeSet::new())
    );
}

#[test]
fn only_contiguous_records_ending_at_the_window_are_valid() {
    let epoch = |from, until| Epoch::new(from, until, 1, String::new());
    assert!(valid(&[], 1, 3));
    assert!(valid(&[epoch(1, 3), epoch(3, 5)], 5, 6));
    assert!(!valid(&[epoch(1, 3), epoch(4, 5)], 5, 6));
    assert!(!valid(&[epoch(1, 3)], 4, 6));
    assert!(!valid(&[epoch(0, 3)], 3, 6));
    assert!(!valid(&[epoch(1, 9)], 9, 6));
}

#[test]
fn rolling_up_condenses_the_oldest_records_and_keeps_them_contiguous() {
    let mut messages = vec![user("system")];
    for index in 0..40 {
        messages.push(user(&format!("request {index}: {}", "detail ".repeat(200))));
        messages.push(assistant(&format!("answer {index}"), &[]));
    }
    let mut context = crate::context::Context {
        from: 41,
        ..Default::default()
    };
    context.digests = (0..4)
        .map(|index| Epoch::new(1 + index * 10, 11 + index * 10, 1, String::new()))
        .collect();
    let before = context.digest_bytes(&messages);
    context.roll_up(&messages, 8_000);
    assert!(context.digest_bytes(&messages) < before);
    assert!(valid(&context.digests, 41, messages.len()));
    assert!(context.digests.iter().any(|epoch| epoch.level > 1));
    assert!(
        context.digests.first().unwrap().level >= context.digests.last().unwrap().level,
        "newer records stay at least as detailed"
    );
}

#[test]
fn costlier_cache_writes_wait_longer_within_the_configured_budget() {
    let context = crate::context::Context::default();
    assert_eq!(context.digest_budget(&[], 400_000, 1.0), 120_000);
    let long = context.digest_budget(&[], 400_000, 2.0);
    assert!((170_000..180_000).contains(&long), "{long}");
    assert_eq!(context.digest_budget(&[], 50_000, 2.0), 50_000);
    assert_eq!(turn_gain(400_000, 1.0), 30_000);
    assert!(turn_gain(400_000, 1.25) > 30_000);
    assert_eq!(turn_gain(4_000, 1.0), 1_000);
}
