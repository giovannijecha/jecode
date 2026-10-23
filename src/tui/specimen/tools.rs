// Tool cards painted by the real card renderer for review in a native terminal:
// cargo test --locked --offline specimen::tools -- --ignored --nocapture
use std::time::{Duration, Instant};

use crate::events::tool_summary;
use crate::json::Value;
use crate::tui::{
    cards::{self, Presentation, Tool},
    line::Line,
    markdown,
    theme::{EMPHASIS, MUTED},
};

const WIDE: usize = 100;
const NARROW: usize = 60;

struct Call {
    name: &'static str,
    arguments: Value,
    result: Option<Value>,
    change: Option<crate::file_change::Change>,
    presentation: Presentation,
}

fn text(value: &str) -> Value {
    Value::string(value)
}

fn finished(name: &'static str, arguments: Value, result: Value, milliseconds: u64) -> Call {
    Call {
        name,
        arguments,
        result: Some(result),
        change: None,
        presentation: Presentation {
            elapsed: Some(Duration::from_millis(milliseconds)),
            ..Presentation::default()
        },
    }
}

fn path(value: &str) -> Value {
    Value::object([("path", text(value))])
}

fn command(value: &str) -> Value {
    Value::object([("command", text(value))])
}

fn update() -> Call {
    let old = "    let limit = 10;\n";
    let new = "    let limit = LOGIN_LIMIT;\n    let window = LOGIN_WINDOW;\n";
    let before = format!(
        "{}pub fn login() {{\n{old}    check(limit);\n}}\n",
        "// Login limits.\n".repeat(39)
    );
    let after = before.replace(old, new);
    let mut call = finished(
        "edit",
        Value::object([
            ("path", text("src/routes/auth.rs")),
            ("old_text", text(old)),
            ("new_text", text(new)),
        ]),
        Value::object([("bytes_written", Value::number(after.len()))]),
        4,
    );
    call.change = crate::file_change::Change::between(Some(&before), &after);
    call
}

fn write() -> Call {
    let content = "use app::limiter;\n\n#[test]\nfn blocks_after_ten_attempts() {\n}\n";
    let mut call = finished(
        "write",
        Value::object([
            ("path", text("tests/limiter.rs")),
            ("content", text(content)),
        ]),
        Value::object([("bytes_written", Value::number(content.len()))]),
        3,
    );
    call.change = crate::file_change::Change::between(None, content);
    call
}

fn sequence() -> Vec<Call> {
    vec![
        finished(
            "read",
            path("src/routes/auth.rs"),
            Value::object([
                (
                    "content",
                    text(
                        "1: use crate::session::Session;\n2:\n3: pub fn login(request: Request) {\n",
                    ),
                ),
                ("lines_returned", Value::number(3)),
                ("offset", Value::number(1)),
            ]),
            12,
        ),
        update(),
        finished(
            "bash",
            command("cargo test --locked limiter"),
            Value::object([
                (
                    "stdout",
                    text(
                        "running 3 tests\ntest limiter::allows ... ok\ntest limiter::blocks ... FAILED\n",
                    ),
                ),
                (
                    "stderr",
                    text("error: test failed, to rerun pass `--lib`\n"),
                ),
                ("exit_code", Value::number(101)),
            ]),
            8400,
        ),
        write(),
        finished(
            "bash",
            command("cargo test --locked limiter"),
            Value::object([
                (
                    "stdout",
                    text(
                        "running 3 tests\ntest limiter::allows ... ok\ntest limiter::blocks ... ok\ntest limiter::resets ... ok\n\ntest result: ok. 3 passed; 0 failed\n",
                    ),
                ),
                ("exit_code", Value::number(0)),
            ]),
            7900,
        ),
    ]
}

// A command long enough to need both header rows at either width.
fn long() -> Call {
    finished(
        "bash",
        command(
            "cargo build --locked --release --target x86_64-pc-windows-gnullvm\ncargo test --locked --offline --workspace -- --include-ignored --test-threads 1",
        ),
        Value::object([
            ("stdout", text("test result: ok. 636 passed; 0 failed\n")),
            ("exit_code", Value::number(0)),
        ]),
        41300,
    )
}

fn states() -> Vec<(&'static str, Call)> {
    let mut running = Call {
        name: "bash",
        arguments: command("cargo build --locked"),
        result: None,
        change: None,
        presentation: Presentation::live(),
    };
    running.presentation.started = Instant::now().checked_sub(Duration::from_millis(2400));
    let mut selected = finished(
        "read",
        path("src/limiter.rs"),
        Value::object([
            (
                "content",
                text("1: pub struct Limiter {\n2:     attempts: u32,\n"),
            ),
            ("lines_returned", Value::number(2)),
            ("offset", Value::number(1)),
        ]),
        9,
    );
    selected.presentation.expanded = true;
    let mut expanded = sequence().remove(1);
    expanded.presentation.expanded = true;
    vec![
        ("Running", running),
        (
            "Background process",
            finished(
                "bash",
                Value::object([
                    ("command", text("npm run dev")),
                    ("wait_ms", Value::number(3000)),
                ]),
                Value::object([
                    ("process_id", text("p1")),
                    ("status", text("running")),
                    ("stdout", text("Listening on http://localhost:3000\n")),
                ]),
                3000,
            ),
        ),
        (
            "Cancelled",
            finished(
                "bash",
                command("cargo build --release"),
                Value::object([
                    ("stdout", text("   Compiling app v0.1.0\n")),
                    ("cancelled", Value::Bool(true)),
                ]),
                5100,
            ),
        ),
        (
            "Timed out",
            finished(
                "bash",
                command("curl http://localhost:3000/health"),
                Value::object([("timed_out", Value::Bool(true))]),
                30000,
            ),
        ),
        (
            "Tool error",
            finished(
                "read",
                path("src/missing.rs"),
                Value::object([("error", text("path not found: src/missing.rs"))]),
                2,
            ),
        ),
        (
            "Truncated capture",
            finished(
                "bash",
                command("cargo test --locked"),
                Value::object([
                    ("stdout", text("test result: ok. 632 passed; 0 failed\n")),
                    ("exit_code", Value::number(0)),
                    ("stdout_bytes", Value::number(91000)),
                    ("stdout_truncated", Value::Bool(true)),
                    ("output_limit_bytes", Value::number(65536)),
                    ("stdout_file", text("output:s1:c4:stdout")),
                ]),
                14200,
            ),
        ),
        ("Long command", long()),
        ("Edit preview", sequence().remove(1)),
        ("Expanded read", selected),
        ("Expanded details", expanded),
    ]
}

fn card(call: &Call, columns: usize) -> Vec<Line> {
    let summary = call
        .result
        .as_ref()
        .map(|result| tool_summary(call.name, result))
        .unwrap_or_default();
    cards::render(
        Tool {
            name: call.name,
            arguments: &call.arguments,
            summary: &summary,
            result: call.result.as_ref(),
            change: call.change.as_ref(),
            presentation: &call.presentation,
        },
        columns,
    )
}

// The completed diff remains visible when another tool starts.
fn turn(rows: &mut Vec<(Line, usize)>, columns: usize) {
    let calls = sequence();
    for line in markdown::render("I'll add a login limiter and test it.", columns) {
        rows.push((line, columns));
    }
    rows.push((Line::default(), columns));
    for (index, call) in [&calls[0], &calls[1], calls.last().unwrap()]
        .into_iter()
        .enumerate()
    {
        if index > 0 {
            rows.push((Line::default(), columns));
        }
        for line in card(call, columns) {
            rows.push((line, columns));
        }
    }
}

fn heading(rows: &mut Vec<(Line, usize)>, title: &str) {
    rows.push((Line::default(), WIDE));
    rows.push((Line::new(title, EMPHASIS.bold()), WIDE));
    rows.push((Line::default(), WIDE));
}

#[test]
#[ignore = "prints the tool card specimen for review in a native terminal"]
fn tools() {
    let mut rows = Vec::new();
    heading(
        &mut rows,
        &format!("Tool cards  ·  one turn at {WIDE} columns"),
    );
    turn(&mut rows, WIDE);
    heading(&mut rows, "Tool cards  ·  states");
    for (title, call) in states() {
        rows.push((Line::new(title, MUTED), WIDE));
        for line in card(&call, WIDE) {
            rows.push((line, WIDE));
        }
        rows.push((Line::default(), WIDE));
    }
    heading(
        &mut rows,
        &format!("Tool cards  ·  the same turn at {NARROW} columns"),
    );
    turn(&mut rows, NARROW);
    rows.push((Line::default(), NARROW));
    rows.push((Line::new("Long command", MUTED), NARROW));
    for line in card(&long(), NARROW) {
        rows.push((line, NARROW));
    }
    for columns in [71, 72] {
        heading(&mut rows, &format!("Tool header  ·  {columns} columns"));
        for line in card(&long(), columns) {
            rows.push((line, columns));
        }
    }
    let painted: String = rows
        .iter()
        .map(|(line, columns)| format!("{}\n", line.paint(*columns)))
        .collect();
    print!("{painted}");
}
