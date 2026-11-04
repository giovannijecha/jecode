use super::*;
use crate::tui::{
    cards::{Presentation, card as render},
    theme::{BADGE_BAD, BADGE_EDIT},
};
use std::time::Duration;

struct Call {
    arguments: Value,
    result: Option<Value>,
    change: Option<Change>,
    presentation: Presentation,
}

/// Each file with its before and after text, or `None` when it failed.
type File<'a> = (&'a str, Option<(&'a str, &'a str)>);

const ERROR: &str = "edit 1: old text was not found; use exact current text from the supplied file context or read the file";

fn call(files: &[File<'_>], running: bool, expanded: bool) -> Call {
    let arguments = Value::object([(
        "files",
        Value::Array(
            files
                .iter()
                .map(|(path, _)| {
                    Value::object([
                        ("path", Value::string(*path)),
                        (
                            "edits",
                            Value::Array(vec![Value::object([
                                ("old_text", Value::string("old")),
                                ("new_text", Value::string("new")),
                            ])]),
                        ),
                    ])
                })
                .collect(),
        ),
    )]);
    let mut changes = Vec::new();
    let entries = files
        .iter()
        .map(|(path, texts)| match texts {
            Some((before, after)) => {
                changes.push((
                    path.to_string(),
                    Change::between(Some(before), after).unwrap(),
                ));
                Value::object([
                    ("path", Value::string(*path)),
                    ("bytes_written", Value::number(after.len())),
                ])
            }
            None => Value::object([
                ("path", Value::string(*path)),
                ("error", Value::string(ERROR)),
            ]),
        })
        .collect();
    Call {
        arguments,
        result: (!running).then(|| Value::object([("files", Value::Array(entries))])),
        change: (!running).then(|| Change::several(changes)).flatten(),
        presentation: Presentation {
            elapsed: Some(Duration::from_millis(400)),
            expanded,
            ..Presentation::default()
        },
    }
}

fn card(call: &Call, columns: usize, keycap: bool) -> Card {
    render(
        Tool {
            name: "edit",
            arguments: &call.arguments,
            summary: "",
            result: call.result.as_ref(),
            change: call.change.as_ref(),
            presentation: &call.presentation,
        },
        columns,
        keycap,
    )
}

fn plain(rows: &[Line]) -> Vec<String> {
    rows.iter().map(Line::plain).collect()
}

fn styled(rows: &[Line], text: &str) -> Option<crate::tui::theme::Style> {
    rows.iter()
        .flat_map(|row| &row.spans)
        .find(|span| span.text.contains(text))
        .map(|span| span.style)
}

const SESSION: &str = "src/auth/session.ts";
const TOKEN: &str = "src/auth/token.ts";
const TEST: &str = "tests/auth/expiry.test.ts";

fn three<'a>() -> Vec<File<'a>> {
    vec![
        (SESSION, Some(("a\nb\nc\n", "a\nB\nc\n"))),
        (TOKEN, Some(("x\n", "x\ny\nz\n"))),
        (TEST, Some(("t\n", "t\nu\n"))),
    ]
}

#[test]
fn three_files_are_one_card_with_a_section_per_file() {
    let finished = call(&three(), false, false);
    for columns in [44, 80, 100] {
        let card = card(&finished, columns, true);
        let rows = plain(&card.rows);
        assert!(
            rows[0].starts_with(" UPDATE  src/auth/session.ts,"),
            "{rows:#?}"
        );
        assert_eq!(card.rows[0].spans[0].background, Some(BADGE_EDIT));
        let fact = "3 files · +4 −1 lines";
        // The fact joins a one-row title when it fits, otherwise hangs on └.
        if columns == 100 {
            assert!(rows[0].ends_with(fact), "{rows:#?}");
        } else {
            assert!(rows.contains(&format!("  └ {fact}")), "{rows:#?}");
        }
        for (path, counts) in [(SESSION, "+1 −1"), (TOKEN, "+2"), (TEST, "+1")] {
            assert!(rows.contains(&format!("  {path}  {counts}")), "{rows:#?}");
        }
        let heading = card
            .rows
            .iter()
            .find(|row| row.plain().starts_with(&format!("  {SESSION}  ")))
            .unwrap();
        assert_eq!(styled(std::slice::from_ref(heading), SESSION), Some(MUTED));
        assert!(rows.iter().any(|row| row.contains("2 + y")));
        assert!(!card.hidden && !rows.iter().any(|row| row.contains("Ctrl+O")));
        assert!(rows.iter().all(|row| text::cells(row) <= columns));
    }
}

#[test]
fn a_one_file_call_keeps_todays_fact_and_running_calls_count_files() {
    let single = call(&three()[..1], false, false);
    let rows = plain(&card(&single, 80, false).rows);
    assert_eq!(rows[0], " UPDATE  src/auth/session.ts  +1 −1 lines");
    assert!(!rows.iter().any(|row| row.trim_start().starts_with(SESSION)));
    let running = card(&call(&three(), true, false), 80, false);
    let rows = plain(&running.rows);
    assert_eq!(rows.last().unwrap(), "  └ 3 files · 0.4s");
    assert_eq!(styled(&running.rows, "3 files"), Some(DIM));
}

#[test]
fn a_failed_file_is_one_row_in_call_order_on_a_failed_card() {
    let mut files = three();
    files[1].1 = None;
    for columns in [44, 80, 100] {
        let collapsed = card(&call(&files, false, false), columns, true);
        let rows = plain(&collapsed.rows);
        assert_eq!(collapsed.rows[0].spans[0].background, Some(BADGE_BAD));
        let fact = rows.iter().find(|row| row.contains("2 files")).unwrap();
        assert!(fact.ends_with("2 files · +2 −1 lines · 1 failed"), "{fact}");
        assert_eq!(styled(&collapsed.rows, " · 1 failed"), Some(BAD));
        let failed = rows
            .iter()
            .position(|row| row.starts_with("  src/auth/token.ts · not changed · edit 1:"))
            .unwrap();
        let session = rows
            .iter()
            .position(|row| row.starts_with(&format!("  {SESSION}  ")))
            .unwrap();
        let test = rows
            .iter()
            .position(|row| row.starts_with(&format!("  {TEST}  ")))
            .unwrap();
        assert!(session < failed && failed < test);
        // Collapsed, the row is clipped; Ctrl+O wraps it under the path.
        assert!(rows[failed].ends_with('…'));
        assert_eq!(text::cells(&rows[failed]), columns);
        let expanded = plain(&card(&call(&files, false, true), columns, false).rows);
        let start = expanded
            .iter()
            .position(|row| row.contains(" · not changed · "))
            .unwrap();
        let wrapped: Vec<_> = expanded[start..]
            .iter()
            .take_while(|row| !row.starts_with(&format!("  {TEST}")))
            .collect();
        assert!(wrapped.len() > 1);
        assert!(wrapped[1..].iter().all(|row| row.starts_with("    ")));
        assert!(wrapped.last().unwrap().ends_with("read the file"));
        assert!(expanded.iter().all(|row| text::cells(row) <= columns));
    }
    let nothing: Vec<File<'_>> = three().into_iter().map(|(path, _)| (path, None)).collect();
    let card = card(&call(&nothing, false, false), 80, true);
    let rows = plain(&card.rows);
    assert!(
        rows.contains(&"  └ No files changed · 3 failed".to_owned()),
        "{rows:#?}"
    );
    assert_eq!(styled(&card.rows, "No files changed"), Some(BAD));
    assert_eq!(card.rows[0].spans[0].background, Some(BADGE_BAD));
    assert_eq!(
        rows.iter()
            .filter(|row| row.contains(" · not changed · "))
            .count(),
        3
    );
}

#[test]
fn nine_files_open_three_diffs_and_count_the_rest_beside_the_keycap() {
    let paths = [
        "src/auth/session.ts",
        "src/auth/index.ts",
        "src/api/client.ts",
        "src/api/retry.ts",
        "src/components/settings/SessionExpiryNotice.tsx",
        "src/workers/refresh.worker.ts",
        "tests/auth/session.test.ts",
        "tests/api/client.test.ts",
        "docs/AUTH.md",
    ];
    let files: Vec<File<'_>> = paths
        .iter()
        .map(|path| {
            (
                *path,
                (*path != "docs/AUTH.md").then_some(("k\nold\n", "k\nnew\n")),
            )
        })
        .collect();
    let finished = call(&files, false, false);
    for columns in [44, 80, 100] {
        let card = card(&finished, columns, true);
        let rows = plain(&card.rows);
        assert!(card.hidden);
        // Collapsed, the title keeps three rows, then an ellipsis; at 100
        // columns all nine paths fit.
        let fact = rows.iter().position(|row| row.contains("8 files")).unwrap();
        assert_eq!(fact, 3, "{rows:#?}");
        assert_eq!(rows[2].ends_with('…'), columns < 100, "{rows:#?}");
        assert_eq!(rows[fact], "  └ 8 files · +8 −8 lines · 1 failed");
        assert_eq!(rows.iter().filter(|row| row.contains("2 + new")).count(), 3);
        let last = rows.last().unwrap();
        assert!(last.starts_with("  +5 diffs · 15 rows"), "{last}");
        assert!(last.ends_with("Ctrl+O  expand"), "{last}");
        let docs = rows
            .iter()
            .position(|row| row.starts_with("  docs/AUTH.md · not changed"))
            .unwrap();
        assert_eq!(docs, rows.len() - 2);
        assert!(
            rows.iter()
                .any(|row| row.starts_with("  src/workers/refresh.worker.ts  +1 −1"))
        );
        assert!(rows.iter().all(|row| text::cells(row) <= columns));
    }
    // Expanded, every diff opens and nothing waits for Ctrl+O.
    let expanded = call(&files, false, true);
    let card = card(&expanded, 80, false);
    assert!(!card.hidden);
    assert_eq!(
        plain(&card.rows)
            .iter()
            .filter(|row| row.contains("2 + new"))
            .count(),
        8
    );
}

#[test]
fn expanded_collapse_keycap_never_splits_a_wrapped_title() {
    let expanded = call(&three(), false, true);
    // A wide row keeps the keycap on the first title row.
    let rows = plain(&card(&expanded, 140, true).rows);
    assert!(rows[0].ends_with("Ctrl+O  collapse"), "{rows:#?}");
    for columns in [44, 80, 100] {
        let card = card(&expanded, columns, true);
        let rows = plain(&card.rows);
        let title = rows
            .iter()
            .take_while(|row| !row.contains("Ctrl+O"))
            .count();
        assert_eq!(rows[title].trim(), "Ctrl+O  collapse", "{rows:#?}");
        // Every row above it belongs to the title: the paths, in order.
        let target = rows[..title]
            .iter()
            .map(|row| row.trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(target.starts_with(&format!("UPDATE  {SESSION}, {TOKEN}, {TEST}")));
        let next = &rows[title + 1];
        if columns == 100 {
            assert!(target.ends_with("3 files · +4 −1 lines"));
            assert!(next.starts_with(&format!("  {SESSION}  ")), "{rows:#?}");
        } else {
            assert_eq!(next, "  └ 3 files · +4 −1 lines");
        }
    }
}
