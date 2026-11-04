use super::*;
use crate::tui::{
    cards::{Presentation, card as render},
    theme::{BADGE_BAD, BADGE_EDIT, BADGE_WARN, WARNING},
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
fn an_emptied_file_changed_though_it_wrote_no_bytes() {
    let emptied: File<'_> = (SESSION, Some(("a\nb\nc\n", "")));
    let files = [emptied, (TOKEN, Some(("x\n", "x\ny\n")))];
    let rows = plain(&card(&call(&files, false, false), 80, false).rows);
    assert!(rows[0].ends_with("  2 files · +1 −3 lines"), "{rows:#?}");
    assert!(rows.contains(&format!("  {SESSION}  −3")), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains("1 − a")), "{rows:#?}");
    // Alone it reads as today's single-file card does.
    let rows = plain(&card(&call(&files[..1], false, false), 80, false).rows);
    assert_eq!(
        rows[0],
        format!(" UPDATE  {SESSION}  −3 lines"),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row.contains("1 − a")), "{rows:#?}");
    // A replacement that changed nothing also reports no bytes, without a diff.
    let mut quiet = call(&files, false, false);
    quiet.result = Some(Value::object([(
        "files",
        Value::Array(vec![
            Value::object([
                ("path", Value::string(SESSION)),
                ("bytes_written", Value::number(0)),
            ]),
            Value::object([
                ("path", Value::string(TOKEN)),
                ("bytes_written", Value::number(4)),
            ]),
        ]),
    )]));
    quiet.change = Change::several(vec![(
        TOKEN.to_string(),
        Change::between(Some("x\n"), "x\ny\n").unwrap(),
    )]);
    let rows = plain(&card(&quiet, 80, false).rows);
    assert!(
        rows.contains(&format!("  {SESSION}  No changes")),
        "{rows:#?}"
    );
    assert!(rows[0].ends_with("  1 file · +1 line"), "{rows:#?}");
}

/// The call with the diffs of `dropped` lost to the diff limits: those files
/// were written, and only their results remain.
fn without_diffs(mut call: Call, dropped: &[&str]) -> Call {
    call.change = call.change.take().and_then(|change| {
        Change::several(
            change
                .files
                .into_iter()
                .filter(|(path, _)| !dropped.contains(&path.as_str()))
                .collect(),
        )
    });
    call
}

#[test]
fn counts_a_lost_diff_cannot_support_are_left_out() {
    let lost = |dropped: &[&str]| {
        plain(
            &card(
                &without_diffs(call(&three(), false, false), dropped),
                100,
                false,
            )
            .rows,
        )
    };
    // No diff kept: the call names how many files, not `0 lines`.
    let rows = lost(&[SESSION, TOKEN, TEST]);
    assert!(rows[0].ends_with("  3 files"), "{rows:#?}");
    assert!(
        rows.contains(&format!("  {SESSION}  Applied 1 replacement")),
        "{rows:#?}"
    );
    // One diff lost: a sum of the others would read as the whole call.
    let rows = lost(&[TOKEN]);
    assert!(rows[0].ends_with("  3 files"), "{rows:#?}");
    assert!(rows.contains(&format!("  {SESSION}  +1 −1")), "{rows:#?}");
    assert!(
        rows.contains(&format!("  {TOKEN}  Applied 1 replacement")),
        "{rows:#?}"
    );
    // Line endings change no lines: the sum is known, and nothing to show.
    let endings = [
        (SESSION, Some(("a\n", "a\r\n"))),
        (TOKEN, Some(("x\n", "x\r\n"))),
    ];
    let rows = plain(&card(&call(&endings, false, false), 100, false).rows);
    assert!(rows[0].ends_with("  2 files"), "{rows:#?}");
    assert!(
        rows.contains(&format!("  {SESSION}  Changed line endings · LF → CRLF")),
        "{rows:#?}"
    );
    let mixed = [endings[0], (TOKEN, Some(("x\n", "x\ny\n")))];
    let rows = plain(&card(&call(&mixed, false, false), 100, false).rows);
    assert!(rows[0].ends_with("  2 files · +1 line"), "{rows:#?}");
}

#[test]
fn the_hidden_diff_row_counts_only_rows_it_knows() {
    let files: Vec<File<'_>> = ["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"]
        .into_iter()
        .map(|path| (path, Some(("k\nold\n", "k\nnew\n"))))
        .collect();
    let last = |call: &Call| {
        let card = card(call, 80, false);
        assert!(card.hidden);
        plain(&card.rows).last().unwrap().clone()
    };
    assert_eq!(last(&call(&files, false, false)), "  +2 diffs · 6 rows");
    let lost = |dropped: &[&str]| last(&without_diffs(call(&files, false, false), dropped));
    assert_eq!(lost(&["d.rs"]), "  +2 diffs");
    assert_eq!(lost(&["d.rs", "e.rs"]), "  +2 diffs");
    let mut few = files[..4].to_vec();
    assert_eq!(
        last(&without_diffs(call(&few, false, false), &["d.rs"])),
        "  +1 diff"
    );
    // Changed line endings keep no rows, and a lone row is one row.
    few[3] = ("d.rs", Some(("x\n", "x\r\n")));
    assert_eq!(last(&call(&few, false, false)), "  +1 diff");
    few[3] = ("d.rs", Some(("x\n", "")));
    assert_eq!(last(&call(&few, false, false)), "  +1 diff · 1 row");
}

/// A call the permission gate stopped before any file was touched.
fn gated(outcome: &str, key: &str, reason: &str, expanded: bool) -> Call {
    let mut gated = call(&three(), false, expanded);
    gated.result = Some(Value::object([
        ("outcome", Value::string(outcome)),
        (key, Value::string(reason)),
    ]));
    gated.change = None;
    gated
}

/// The card's words with wrapping and the tree gutter taken out.
fn said(rows: &[String]) -> String {
    rows.iter()
        .map(|row| row.trim_start().trim_start_matches(['│', '└']).trim())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn a_call_that_never_ran_is_stopped_and_says_why() {
    let refused = "Not available at the Read permission level: editing src/auth/session.ts, src/auth/token.ts, tests/auth/expiry.test.ts. Read allows reading and analysis inside the working folder; explain what you need and the user can change the level with /permissions.";
    let denied = "The user denied editing src/auth/session.ts, src/auth/token.ts, tests/auth/expiry.test.ts; nothing ran. Continue without it or explain what it was for.";
    let stopped = "Tool was not executed: the turn stopped before a permission decision";
    for (outcome, key, reason) in [
        ("unavailable", "reason", refused),
        ("denied", "reason", denied),
        ("not_executed", "error", stopped),
    ] {
        for columns in [44, 80, 100] {
            let collapsed = card(&gated(outcome, key, reason, false), columns, true);
            let rows = plain(&collapsed.rows);
            assert_eq!(
                collapsed.rows[0].spans[0].background,
                Some(BADGE_WARN),
                "{outcome}: {rows:#?}"
            );
            assert!(!rows.iter().any(|row| row.contains("No ")), "{rows:#?}");
            let opening = reason.split(' ').next().unwrap();
            assert_eq!(styled(&collapsed.rows, opening), Some(WARNING), "{rows:#?}");
            assert!(rows.iter().all(|row| text::cells(row) <= columns));
            // Clipped, the rest waits for Ctrl+O; expanded, all of it shows.
            let whole = said(&rows).contains(reason);
            assert_eq!(collapsed.hidden, !whole, "{rows:#?}");
            assert_eq!(rows.iter().any(|row| row.contains("Ctrl+O")), !whole);
            let opened = plain(&card(&gated(outcome, key, reason, true), columns, false).rows);
            assert!(said(&opened).contains(reason), "{opened:#?}");
            assert!(opened.iter().all(|row| text::cells(row) <= columns));
        }
    }
    // Control characters in a named path stay out of every row, title join
    // included.
    let hostile = "The user denied editing a\x1b]0;x\x07.rs\nb.rs; nothing ran";
    for columns in [80, 200] {
        for expanded in [false, true] {
            let call = gated("denied", "reason", hostile, expanded);
            for row in plain(&card(&call, columns, false).rows) {
                assert!(!row.contains(['\x1b', '\x07', '\n']), "{row:?}");
            }
        }
    }
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
