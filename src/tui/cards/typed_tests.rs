use super::*;
use crate::tui::motion;
use crate::tui::theme::{
    BAD, BADGE_BAD, BADGE_EDIT, BADGE_IDLE, BADGE_READ, BADGE_SHELL, BADGE_TEXT, BADGE_WARN, BODY,
    LIT, Surface,
};
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

/// The title badge of a finished call: its ink and surface.
fn badge(rows: &[Line]) -> (Style, Option<Surface>) {
    (rows[0].spans[0].style, rows[0].spans[0].background)
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
    for (name, arguments, label, family) in [
        ("read", object([("path", "a.rs")]), " READ ", BADGE_READ),
        ("write", object([("path", "a.rs")]), " WRITE ", BADGE_EDIT),
        ("bash", object([("command", "ls")]), " BASH ", BADGE_SHELL),
    ] {
        let result = Value::object([("exit_code", Value::number(0))]);
        let rows = card(tool(name, &arguments, &result, &shown), 80, false).rows;
        let spans = &rows[0].spans;
        // Done is the family surface under the badge; the target stays BODY.
        assert_eq!(spans[0].text, label);
        assert_eq!(badge(&rows), (BADGE_TEXT.bold(), Some(family)));
        assert_eq!(spans[1].style, BODY);
        assert_eq!(
            spans.last().unwrap().style,
            if name == "bash" { BODY } else { MUTED }
        );
    }
}

#[test]
fn every_running_tool_pulses_only_its_marker_and_keeps_the_timer_and_note() {
    let arguments = object([("path", "fixture.rs"), ("command", "cargo build")]);
    for (name, label) in [
        ("read", "READ"),
        ("write", "WRITE"),
        ("edit", "UPDATE"),
        ("bash", "BASH"),
        ("process", "PROCESS"),
        ("checkpoint", "CHECKPOINT"),
        ("web", "WEB"),
        ("browser", "BROWSER"),
        ("future_tool", "FUTURE_TOOL"),
    ] {
        let mut bands = Vec::new();
        for frame in 0..label.len() + 8 {
            motion::set_frame(frame);
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
            // The quiet badge carries the shimmer across its name only.
            let spans = &rows[0].spans;
            let width = spans
                .iter()
                .take_while(|span| span.background == Some(BADGE_IDLE))
                .count();
            assert_eq!(spans[0].text, " ");
            assert_eq!(spans[width - 1].text, " ");
            let shimmer: Vec<_> = spans[1..width - 1]
                .iter()
                .map(|span| (span.text.clone(), span.style))
                .collect();
            let expected: Vec<_> = motion::shimmer(label, frame, true)
                .spans
                .into_iter()
                .map(|span| (span.text, span.style))
                .collect();
            assert_eq!(shimmer, expected, "{name}, frame {frame}");
            assert!(
                spans[width..].iter().all(|span| span.background.is_none()),
                "{name}"
            );
            assert_eq!(spans[width].style, BODY, "{name}");
            bands.push(shimmer);
            let expected = crate::tui::activity::duration(shown.elapsed.unwrap());
            if matches!(name, "read" | "write" | "edit") {
                // File facts join the one-row title.
                assert_eq!(rows.len(), 1, "{name}");
                assert_eq!(
                    rows[0].plain(),
                    format!(" {label}  fixture.rs  {expected} · checkpoint rc-fixture"),
                    "{name}"
                );
            } else {
                assert_eq!(
                    rows.last().unwrap().plain(),
                    format!("  └ Running · {expected} · checkpoint rc-fixture")
                );
            }
            assert!(!plain(&rows).join("").contains(['▰', '▱', '●']));
        }
        motion::set_frame(0);
        bands.dedup();
        assert!(bands.len() > 1, "{name} never moved its shimmer");
    }
}

#[test]
fn returned_tools_keep_their_static_outcome_marker() {
    let arguments = object([("command", "cargo build")]);
    for (result, expected) in [
        (
            Value::object([("exit_code", Value::number(0))]),
            (BADGE_TEXT.bold(), Some(BADGE_SHELL)),
        ),
        (
            object([("error", "failed")]),
            (BADGE_TEXT.bold(), Some(BADGE_BAD)),
        ),
        (
            Value::object([("cancelled", Value::Bool(true))]),
            (BADGE_TEXT.bold(), Some(BADGE_WARN)),
        ),
        (
            object([("process_id", "p1"), ("status", "running")]),
            (LIT.bold(), Some(BADGE_IDLE)),
        ),
    ] {
        for (elapsed, frame) in [(0, 0), (480, 5)] {
            motion::set_frame(frame);
            let shown = presentation(elapsed, false);
            let rows = card(tool("bash", &arguments, &result, &shown), 80, false).rows;
            assert_eq!(rows[0].spans[0].text, " BASH ");
            assert_eq!(badge(&rows), expected);
        }
    }
    motion::set_frame(0);
}

#[test]
fn compact_routine_titles_use_one_clipped_row() {
    let arguments = object([(
        "command",
        "cargo build --locked --release --target x86_64-pc-windows-gnullvm\ncargo test --locked",
    )]);
    let result = Value::object([("exit_code", Value::number(0))]);
    let shown = presentation(0, false);
    let rows = plain(&card(tool("bash", &arguments, &result, &shown), 60, false).rows);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].starts_with(" BASH  cargo build --locked --release"));
    assert!(rows[0].ends_with('…'));
    assert_eq!(rows[1], "  └ Done");
    let long = object([("command", &"word ".repeat(60))]);
    let compact = plain(&card(tool("bash", &long, &result, &shown), 60, false).rows);
    assert_eq!(compact.len(), 2);
    assert!(compact[0].ends_with('…'));
    let expanded = presentation(0, true);
    let full = card(tool("bash", &long, &result, &expanded), 60, false).rows;
    let title: Vec<_> = full
        .iter()
        .filter(|row| row.origin.is_some_and(|origin| origin.line == 0))
        .collect();
    assert!(title.len() > 1);
    assert!(!title.iter().any(|row| row.plain().contains('…')));
    assert!(title[1..].iter().all(|row| row.plain().starts_with("  ")));
    let retained = title.iter().map(|row| row.plain()).collect::<String>();
    assert_eq!(
        retained.split_whitespace().collect::<String>(),
        format!("BASH {}", "word ".repeat(60).trim())
            .split_whitespace()
            .collect::<String>()
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
    assert!(rows[1].starts_with("  └ test result: ok. 3 passed · 6.8s  +2 lines"));
    assert!(rows.iter().any(|row| row.ends_with("Ctrl+O  expand")));
    assert!(rows.iter().all(|row| crate::tui::text::cells(row) <= 60));
    let quick = presentation(400, false);
    let empty = Value::object([("exit_code", Value::number(0))]);
    let rows = plain(&super::card(tool("bash", &arguments, &empty, &quick), 60, true).rows);
    assert_eq!(rows, [" BASH  cargo test", "  └ Done"]);
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
            "  │ Exit 101 · error[E0425]: missing · a.rs",
            "  │ note",
            "  └ more"
        ]
    );
    assert!(
        rows[1]
            .spans
            .iter()
            .any(|span| span.style == BAD && span.text.contains("Exit 101"))
    );
    let missing = Value::object([(
        "error",
        Value::string("Could not resolve path \"a.rs\": No such file (os error 2)"),
    )]);
    let path = object([("path", "a.rs")]);
    let rows = card(tool("read", &path, &missing, &shown), 80, false).rows;
    assert_eq!(plain(&rows), [" READ  a.rs  Not found"]);
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
    assert_eq!(plain(&rows), [" READ  a.rs  Not found"]);

    let command = object([("command", "pwd")]);
    let failed = Value::object([(
        "error",
        Value::string("Invalid working directory: No such file (os error 2)"),
    )]);
    let rows = card(tool("bash", &command, &failed, &shown), 80, false).rows;
    assert_eq!(
        plain(&rows)[1],
        "  └ Invalid working directory: No such file (os error 2)"
    );

    let unix_error = Value::object([(
        "error",
        Value::string("Could not resolve path \"a.rs\": No such process (os error 3)"),
    )]);
    if cfg!(unix) {
        let rows = card(tool("read", &path, &unix_error, &shown), 80, false).rows;
        assert_eq!(
            plain(&rows),
            [" READ  a.rs  Could not resolve path \"a.rs\": No such process (os error 3)"]
        );
    }
    let windows_error = Value::object([(
        "error",
        Value::string("Could not resolve path \"a.rs\": The path was not found (os error 3)"),
    )]);
    if cfg!(windows) {
        let rows = card(tool("read", &path, &windows_error, &shown), 80, false).rows;
        assert_eq!(plain(&rows), [" READ  a.rs  Not found"]);
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
        ["  └ governor - Rust · 18 KiB · docs.rs  +2 lines"]
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
    assert_eq!(rows[0], " BROWSER  screenshot");
    assert_eq!(rows[1], "  │ tab-1");
    assert_eq!(
        rows[2],
        "  └ Screenshot 1280×720 · tmp:browser/shot.png  +7 lines"
    );
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
    assert!(rows[0].starts_with(" WORK  Activity  3 calls"));
    assert!(rows.join("\n").contains("│ 3 files read"));
    assert!(rows[0].ends_with("Ctrl+O  expand"));
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
        assert!(rows[0].starts_with(&format!(" READ  {path}")), "{rows:?}");
        assert!(rows.join("\n").contains(&format!("{lines} lines")));
        assert!(
            rows.iter().any(|row| row
                .strip_prefix("  └ ")
                .is_some_and(|row| row.trim() == "1   x")),
            "{rows:?}"
        );
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
        assert!(plain(&page.rows)[0].starts_with(" READ  big.rs"));
        assert!(plain(&page.rows).join("\n").contains("40 lines"));
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
