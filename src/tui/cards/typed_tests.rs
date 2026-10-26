use super::*;
use crate::tui::theme::{ACCENT, EMPHASIS, KEYWORD};
use std::time::Duration;

fn presentation(milliseconds: u64, expanded: bool) -> Presentation {
    Presentation {
        elapsed: Some(Duration::from_millis(milliseconds)),
        expanded,
        ..Presentation::default()
    }
}

fn tool<'a>(
    name: &'a str,
    arguments: &'a Value,
    result: &'a Value,
    presentation: &'a Presentation,
) -> Tool<'a> {
    Tool {
        name,
        arguments,
        summary: "complete",
        result: Some(result),
        change: None,
        presentation,
    }
}

fn plain(rows: &[Line]) -> Vec<String> {
    rows.iter().map(Line::plain).collect()
}

fn object<const N: usize>(fields: [(&str, &str); N]) -> Value {
    Value::object(fields.map(|(key, value)| (key, Value::string(value))))
}

fn read(path: &str, lines: usize) -> (Value, Value) {
    (
        object([("path", path)]),
        Value::object([
            ("content", Value::string("1: x\n")),
            ("lines_returned", Value::number(lines)),
            ("offset", Value::number(1)),
        ]),
    )
}

#[test]
fn names_take_their_family_colour_while_the_marker_keeps_the_status() {
    let shown = presentation(0, false);
    for (name, arguments, family) in [
        ("read", object([("path", "a.rs")]), ACCENT),
        ("write", object([("path", "a.rs")]), KEYWORD),
        ("bash", object([("command", "ls")]), TOOL_ACCENT),
    ] {
        let result = Value::object([("exit_code", Value::number(0))]);
        let rows = card(tool(name, &arguments, &result, &shown), 80, false).rows;
        let spans = &rows[0].spans;
        assert_eq!(spans[0].style, GOOD);
        assert_eq!(spans[1].style, family.bold());
        assert_eq!(spans[2].style, EMPHASIS);
        assert_eq!(spans.last().unwrap().style, family.bold());
    }
}

#[test]
fn every_running_tool_pulses_only_its_marker_and_keeps_the_timer_and_note() {
    use crate::tui::theme::TOOL_PULSE;
    let arguments = object([("path", "fixture.rs"), ("command", "cargo build")]);
    for name in [
        "read",
        "write",
        "edit",
        "bash",
        "process",
        "checkpoint",
        "web",
        "browser",
        "future_tool",
    ] {
        for (frame, shade) in TOOL_PULSE.iter().enumerate() {
            let shown = Presentation {
                elapsed: Some(Duration::from_millis(240 * frame as u64)),
                note: Some("checkpoint rc-fixture".into()),
                ..Presentation::default()
            };
            let rows = card(
                Tool {
                    name,
                    arguments: &arguments,
                    summary: "",
                    result: None,
                    change: None,
                    presentation: &shown,
                },
                80,
                false,
            )
            .rows;
            assert_eq!(rows[0].spans[0].text, "● ");
            assert_eq!(rows[0].spans[0].style, *shade, "{name}, frame {frame}");
            let expected = crate::tui::activity::duration(shown.elapsed.unwrap());
            assert_eq!(
                rows[1].plain(),
                format!("  ⎿ {expected} · checkpoint rc-fixture")
            );
            assert!(!plain(&rows).join("").contains(['▰', '▱']));
        }
    }
}

#[test]
fn returned_tools_keep_their_static_outcome_marker() {
    let arguments = object([("command", "cargo build")]);
    for (result, expected) in [
        (Value::object([("exit_code", Value::number(0))]), GOOD),
        (object([("error", "failed")]), BAD),
        (Value::object([("cancelled", Value::Bool(true))]), WARNING),
        (
            object([("process_id", "p1"), ("status", "running")]),
            TOOL_ACCENT,
        ),
    ] {
        for elapsed in [0, 480] {
            let shown = presentation(elapsed, false);
            let rows = card(tool("bash", &arguments, &result, &shown), 80, false).rows;
            assert_eq!(rows[0].spans[0].style, expected);
        }
    }
}

#[test]
fn titles_wrap_on_words_under_the_open_paren_and_compact_titles_close_with_an_ellipsis() {
    let arguments = object([(
        "command",
        "cargo build --locked --release --target x86_64-pc-windows-gnullvm\ncargo test --locked",
    )]);
    let result = Value::object([("exit_code", Value::number(0))]);
    let shown = presentation(0, false);
    let rows = plain(&card(tool("bash", &arguments, &result, &shown), 60, false).rows);
    assert_eq!(rows[0], "● Bash(cargo build --locked --release --target");
    assert_eq!(rows[1], "       x86_64-pc-windows-gnullvm");
    assert_eq!(rows[2], "       cargo test --locked)");
    let long = object([("command", &"word ".repeat(60))]);
    let compact = plain(&card(tool("bash", &long, &result, &shown), 60, false).rows);
    assert!(compact[2].ends_with("…)") && !compact[3].starts_with("    "));
    let expanded = presentation(0, true);
    let full = card(tool("bash", &long, &result, &expanded), 60, false).rows;
    assert!(
        full.iter()
            .filter(|row| row.origin.unwrap().line == 0)
            .count()
            > 3
    );
}

#[test]
fn commands_name_their_result_line_and_count_the_rest_with_a_keycap() {
    let arguments = object([("command", "cargo test")]);
    let result = Value::object([
        ("exit_code", Value::number(0)),
        (
            "stdout",
            Value::string("running 3 tests\n\ntest a ... ok\ntest result: ok. 3 passed\n"),
        ),
    ]);
    let shown = presentation(6800, false);
    let card = card(tool("bash", &arguments, &result, &shown), 60, true);
    let rows = plain(&card.rows);
    assert!(card.hidden);
    assert_eq!(rows[1], "  ⎿ test result: ok. 3 passed · 6.8s");
    assert!(rows[2].starts_with("    … 2 more lines") && rows[2].ends_with("Ctrl+O  expand"));
    assert_eq!(rows[2].chars().count(), 60);
    let quick = presentation(400, false);
    let empty = Value::object([("exit_code", Value::number(0))]);
    let rows = plain(&super::card(tool("bash", &arguments, &empty, &quick), 60, true).rows);
    assert_eq!(rows, ["● Bash(cargo test)", "  ⎿ Done"]);
}

#[test]
fn failures_lead_with_the_exit_and_first_error_in_the_failure_colour() {
    let arguments = object([("command", "cargo test")]);
    let result = Value::object([
        ("exit_code", Value::number(101)),
        (
            "stderr",
            Value::string("error[E0425]: missing\n  --> a.rs\n\nnote\nmore\n"),
        ),
    ]);
    let shown = presentation(4200, false);
    let rows = card(tool("bash", &arguments, &result, &shown), 80, false).rows;
    assert_eq!(
        plain(&rows)[1..],
        [
            "  ⎿ Exit 101 · error[E0425]: missing",
            "      --> a.rs",
            "    note",
            "    … 1 more line"
        ]
    );
    assert_eq!(rows[1].spans[0].style, BAD);
    let missing = Value::object([(
        "error",
        Value::string("Could not resolve path \"a.rs\": No such file (os error 2)"),
    )]);
    let path = object([("path", "a.rs")]);
    let rows = card(tool("read", &path, &missing, &shown), 80, false).rows;
    assert_eq!(plain(&rows)[1], "  ⎿ Not found");
}

#[test]
fn only_missing_file_paths_receive_the_short_not_found_summary() {
    let shown = presentation(0, false);
    let path = object([("path", "a.rs")]);
    let missing = Value::object([(
        "error",
        Value::string("Could not resolve path \"a.rs\": No such file (os error 2)"),
    )]);
    let rows = card(tool("read", &path, &missing, &shown), 80, false).rows;
    assert_eq!(plain(&rows)[1], "  ⎿ Not found");

    let command = object([("command", "pwd")]);
    let failed = Value::object([(
        "error",
        Value::string("Invalid working directory: No such file (os error 2)"),
    )]);
    let rows = card(tool("bash", &command, &failed, &shown), 80, false).rows;
    assert_eq!(
        plain(&rows)[1],
        "  ⎿ Invalid working directory: No such file (os error 2)"
    );

    let unix_error = Value::object([(
        "error",
        Value::string("Could not resolve path \"a.rs\": No such process (os error 3)"),
    )]);
    if cfg!(unix) {
        let rows = card(tool("read", &path, &unix_error, &shown), 80, false).rows;
        assert_eq!(
            plain(&rows)[1],
            "  ⎿ Could not resolve path \"a.rs\": No such process (os error 3)"
        );
    }
    let windows_error = Value::object([(
        "error",
        Value::string("Could not resolve path \"a.rs\": The path was not found (os error 3)"),
    )]);
    if cfg!(windows) {
        let rows = card(tool("read", &path, &windows_error, &shown), 80, false).rows;
        assert_eq!(plain(&rows)[1], "  ⎿ Not found");
    }
}

#[test]
fn pages_and_captures_summarise_what_came_back() {
    let shown = presentation(600, false);
    let fetch = object([("action", "fetch"), ("url", "https://docs.rs/governor/")]);
    let page = Value::object([
        ("final_url", Value::string("https://docs.rs/governor/")),
        ("status", Value::number(200)),
        ("bytes", Value::number(18402)),
        ("title", Value::string("governor - Rust")),
        (
            "preview",
            Value::string("# Crate governor\n\nRate limiting.\n"),
        ),
    ]);
    let rows = plain(&card(tool("web", &fetch, &page, &shown), 80, false).rows);
    assert_eq!(
        rows[1..],
        [
            "  ⎿ governor - Rust · 18 KiB · docs.rs",
            "    … 2 more lines"
        ]
    );
    let capture = object([("action", "screenshot"), ("tab_id", "tab-1")]);
    let image = Value::object([
        ("screenshot_path", Value::string("tmp:browser/shot.png")),
        (
            "attachment",
            Value::object([
                ("width", Value::number(1280)),
                ("height", Value::number(720)),
            ]),
        ),
    ]);
    let rows = plain(&card(tool("browser", &capture, &image, &shown), 80, false).rows);
    assert_eq!(rows[1], "  ⎿ Screenshot 1280×720 · tmp:browser/shot.png");
}

#[test]
fn read_chapter_counts_files_and_expands_to_individual_cards() {
    let shown = presentation(0, false);
    let calls = [
        read("src/routes/auth.rs", 212),
        read("tests/auth.rs", 120),
        read("src/limiter.rs", 88),
    ];
    let tools: Vec<_> = calls
        .iter()
        .map(|(arguments, result)| tool("read", arguments, result, &shown))
        .collect();
    assert!(tools.iter().all(chapter_groupable));
    let chapter = chapter_header(&tools, 80, true);
    let rows = plain(&chapter.rows);
    assert!(chapter.hidden);
    assert_eq!(rows[0], "● Activity (3 calls)");
    assert!(rows.join("\n").contains("⎿ 3 files read"));
    assert!(rows.join("\n").contains("Ctrl+O"));
    let expanded = presentation(0, true);
    let cards: Vec<_> = calls
        .iter()
        .map(|(arguments, result)| card(tool("read", arguments, result, &expanded), 80, false))
        .collect();
    for (card, (path, lines)) in cards.iter().zip([
        ("src/routes/auth.rs", 212),
        ("tests/auth.rs", 120),
        ("src/limiter.rs", 88),
    ]) {
        let rows = plain(&card.rows);
        assert!(rows[0].contains(&format!("Read({path})")), "{rows:?}");
        assert!(rows.join("\n").contains(&format!("Read {lines} lines")));
        assert!(rows.iter().any(|row| row.trim() == "1   x"));
    }
    let pages = [read("big.rs", 40), read("big.rs", 40)];
    let tools: Vec<_> = pages
        .iter()
        .map(|(arguments, result)| tool("read", arguments, result, &shown))
        .collect();
    let chapter = chapter_header(&tools, 80, false);
    let chapter_text = plain(&chapter.rows).join("\n");
    assert!(chapter_text.contains("2 calls"));
    assert!(chapter_text.contains("1 file read"));
    let page_cards: Vec<_> = pages
        .iter()
        .map(|(arguments, result)| card(tool("read", arguments, result, &expanded), 80, false))
        .collect();
    assert_eq!(page_cards.len(), 2);
    for page in &page_cards {
        assert!(plain(&page.rows)[0].contains("Read(big.rs)"));
        assert!(plain(&page.rows).join("\n").contains("Read 40 lines"));
    }
    let failed = Value::object([("error", Value::string("denied"))]);
    let failed = tool("read", &calls[0].0, &failed, &shown);
    assert!(chapter_groupable(&failed));
    assert!(
        chapter_exception(&failed, 80)
            .iter()
            .any(|row| row.plain().contains("denied"))
    );
}
