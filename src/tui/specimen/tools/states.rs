use super::*;

pub(super) fn states() -> Vec<(&'static str, Call)> {
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
                Value::object([(
                    "error",
                    text(
                        "Could not resolve path \"src/missing.rs\": The system cannot find the file specified. (os error 2)",
                    ),
                )]),
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
        ("Failed command", failed()),
        ("Web page", web()),
        (
            "Browser screenshot",
            finished(
                "browser",
                Value::object([("action", text("screenshot")), ("tab_id", text("tab-1"))]),
                Value::object([
                    ("tab_id", text("tab-1")),
                    ("screenshot_path", text("tmp:browser/capture-1.png")),
                    ("view", text("image")),
                    (
                        "attachment",
                        Value::object([
                            ("width", Value::number(1280)),
                            ("height", Value::number(720)),
                        ]),
                    ),
                ]),
                900,
            ),
        ),
        (
            "Browser click",
            finished(
                "browser",
                Value::object([
                    ("action", text("click")),
                    ("selector", text("button#submit")),
                ]),
                Value::object([("clicked", Value::Bool(true)), ("tab_id", text("tab-1"))]),
                300,
            ),
        ),
        ("Long command", long()),
        ("Edit preview", sequence().remove(1)),
        ("Expanded read", selected),
        ("Expanded details", expanded),
    ]
}
