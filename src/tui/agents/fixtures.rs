use super::{list, number, set, value};
use crate::json::Value;

pub(super) fn record(number: usize, state: &str) -> Value {
    let mut record = Value::object([
        ("id", Value::string(format!("a{number}"))),
        ("number", Value::number(number)),
        ("title", Value::string(format!("Search part {number}"))),
        ("state", Value::string(state)),
        ("workspace", Value::string("isolated")),
        ("directory", Value::string("fixture")),
        ("sent_version", Value::number(2)),
        ("received_version", Value::number(1)),
        ("instructions", Value::string("Implement search")),
        ("pending_instruction", Value::string("Preserve order")),
        ("activity", Value::string("Checking search")),
        (
            "tools",
            Value::Array(vec![Value::object([
                ("id", Value::string("c1")),
                ("name", Value::string("edit")),
                ("state", Value::string("done")),
                ("text", Value::string("src/search.rs")),
                (
                    "arguments",
                    Value::object([("path", Value::string("src/search.rs"))]),
                ),
                ("result", Value::object([])),
                (
                    "change",
                    Value::object([
                        ("added", Value::number(3)),
                        ("removed", Value::number(1)),
                        ("created", Value::Bool(false)),
                    ]),
                ),
            ])]),
        ),
        ("files", Value::Array(vec![Value::string("src/search.rs")])),
        (
            "checks",
            Value::Array(vec![Value::string("bash: search check passed")]),
        ),
        (
            "report",
            Value::object([
                ("version", Value::number(1)),
                ("text", Value::string("Search implemented")),
            ]),
        ),
        ("reviewed", Value::Bool(false)),
        ("error", Value::Null),
    ]);
    sync_log(&mut record);
    record
}

pub(super) fn sync_log(record: &mut Value) {
    let mut entries = vec![Value::object([
        ("kind", Value::string("assignment")),
        ("version", Value::number(number(record, "received_version"))),
        ("instructions", Value::string(value(record, "instructions"))),
    ])];
    entries.extend((0..list(record, "tools").len()).map(|index| {
        Value::object([
            ("kind", Value::string("tool")),
            ("index", Value::number(index)),
        ])
    }));
    entries.extend((0..list(record, "updates").len()).map(|index| {
        Value::object([
            ("kind", Value::string("message")),
            ("index", Value::number(index)),
        ])
    }));
    if matches!(value(record, "state"), "returned" | "failed")
        && let Some(version) = record
            .get("report")
            .and_then(|report| report.get("version"))
    {
        entries.push(Value::object([
            ("kind", Value::string("returned")),
            ("version", version.clone()),
        ]));
    }
    set(record, "log", Value::Array(entries));
}
