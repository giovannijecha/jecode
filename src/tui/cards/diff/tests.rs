use super::*;
use crate::tui::theme::{
    DIFF_ADDED_STRONG, DIFF_ADDED_SURFACE, DIFF_REMOVED_STRONG, DIFF_REMOVED_SURFACE, Surface,
};

fn highlighted(rows: &[Line], surface: Surface) -> String {
    rows.iter()
        .flat_map(|row| &row.spans)
        .filter(|span| span.background == Some(surface))
        .map(|span| span.text.as_str())
        .collect()
}

#[test]
fn context_changes_keep_coordinates_and_syntax_on_tinted_rows() {
    let change = Change::between(
        Some("fn main() {\n    let limit = 3;\n}\n"),
        "fn main() {\n    let limit = 12;\n}\n",
    )
    .unwrap();
    let body = render(&change, "main.rs", 80, false);
    assert_eq!(
        body.rows.iter().map(Line::plain).collect::<Vec<_>>(),
        [
            "    1    fn main() {",
            "    2 −      let limit = 3;",
            "    2 +      let limit = 12;",
            "    3    }"
        ]
    );
    assert_eq!(body.rows[0].background, None);
    assert_eq!(body.rows[1].background, Some(DIFF_REMOVED_SURFACE));
    assert_eq!(body.rows[2].background, Some(DIFF_ADDED_SURFACE));
    assert_eq!(body.rows[3].background, None);
    assert_eq!(highlighted(&body.rows, DIFF_REMOVED_STRONG), "3");
    assert_eq!(highlighted(&body.rows, DIFF_ADDED_STRONG), "12");
    assert!(
        body.rows[0]
            .spans
            .iter()
            .skip(1)
            .all(|span| span.style == TOOL_DIM)
    );
    assert!(
        body.rows[2]
            .spans
            .iter()
            .any(|span| span.text == "let" && span.style == crate::tui::theme::KEYWORD)
    );
    let painted = body.rows[2].paint(80);
    let (margin, rest) = painted.split_once("    ").unwrap();
    assert!(margin.contains(&Surface::TERMINAL.to_string()));
    assert!(rest.contains(&DIFF_ADDED_SURFACE.to_string()));
    assert!(rest.contains(&DIFF_ADDED_STRONG.to_string()));
    assert!(painted.ends_with(&format!(
        "\x1b[0m\x1b[{}m{}\x1b[0m",
        DIFF_ADDED_SURFACE,
        " ".repeat(80 - text::cells(&body.rows[2].plain()))
    )));
}

#[test]
fn long_previews_keep_both_sides_and_expand_every_hunk() {
    let old = (0..90).map(|n| format!("line {n}\n")).collect::<String>();
    let new = old
        .replace(
            &(6..19).map(|n| format!("line {n}\n")).collect::<String>(),
            &(6..19)
                .map(|n| format!("changed {n}\n"))
                .collect::<String>(),
        )
        .replace("line 55\n", "changed 55\n")
        .replace("line 81\n", "changed 81\n");
    let change = Change::between(Some(&old), &new).unwrap();
    let preview = render(&change, "notes.txt", 80, false);
    assert!(preview.hidden > 0);
    assert!(preview.rows.iter().any(|row| {
        row.spans
            .iter()
            .any(|span| span.background == Some(DIFF_REMOVED_STRONG))
    }));
    assert!(preview.rows.iter().any(|row| {
        row.spans
            .iter()
            .any(|span| span.background == Some(DIFF_ADDED_STRONG))
    }));
    let full = render(&change, "notes.txt", 80, true);
    assert_eq!(full.hidden, 0);
    let text = full
        .rows
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("changed 55") && text.contains("changed 81") && text.contains(" ⋮"));
    let separator = full
        .rows
        .iter()
        .find(|row| row.plain().contains('⋮'))
        .unwrap();
    assert_eq!(separator.plain().chars().nth(7), Some('⋮'));
    assert_eq!(separator.background, None);
    assert!(separator.spans.iter().all(|span| span.style == TOOL_DIM));
    assert_eq!(preview.hidden_hunks, change.hunks.len());
    assert_eq!(full.hidden_hunks, 0);
    assert!(full.rows.len() > preview.rows.len());
}

#[test]
fn compact_preview_counts_wholly_hidden_hunks() {
    let old = (0..60).map(|n| format!("line {n}\n")).collect::<String>();
    let new = old
        .replace("line 5\n", "changed 5\n")
        .replace("line 30\n", "changed 30\n")
        .replace("line 55\n", "changed 55\n");
    let change = Change::between(Some(&old), &new).unwrap();
    assert_eq!(change.hunks.len(), 3);
    let body = render(&change, "notes.txt", 80, false);
    assert_eq!(body.hidden_hunks, 2);
    assert_eq!(body.hidden, change.hunks[1].len() + change.hunks[2].len());
}

#[test]
fn context_between_deletion_and_addition_cannot_hide_the_added_side() {
    let lines = |prefix: &str, count| {
        (0..count)
            .map(|n| format!("{prefix} {n}\n"))
            .collect::<String>()
    };
    let prefix = lines("before", 3);
    let middle = lines("unchanged", 6);
    let suffix = lines("after", 3);
    let old = format!("{prefix}{}{middle}{suffix}", lines("removed", 6));
    let new = format!("{prefix}{middle}{}{suffix}", lines("added", 6));
    let change = Change::between(Some(&old), &new).unwrap();
    assert_eq!(change.hunks.len(), 1);
    let body = render(&change, "notes.txt", 80, false);
    assert!(body.rows.len() <= 14);
    for surface in [DIFF_REMOVED_SURFACE, DIFF_ADDED_SURFACE] {
        assert!(
            body.rows
                .iter()
                .filter(|row| row.background == Some(surface))
                .count()
                >= 3
        );
    }
    assert!(body.hidden > 0);
}

#[test]
fn wrapped_code_stays_complete_with_empty_continuation_gutters() {
    let source =
        "    let message = \"Paths and Unicode é 東京 stay available when the code wraps.\";";
    let change = Change::between(None, &format!("{source}\n")).unwrap();
    for width in [20, 40, 80, 120] {
        let body = render(&change, "source.rs", width, true);
        let rows: Vec<_> = body
            .rows
            .iter()
            .filter(|row| row.background == Some(DIFF_ADDED_SURFACE))
            .collect();
        let retained: String = rows
            .iter()
            .flat_map(|row| row.spans.iter().skip(2))
            .map(|span| span.text.as_str())
            .collect();
        assert_eq!(retained, source);
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= width));
        for row in rows.iter().skip(1) {
            assert!(row.spans[1].text.chars().all(|ch| ch == ' '));
        }
        assert!(
            rows.iter()
                .all(|row| row.origin.unwrap().line == rows[0].origin.unwrap().line)
        );
    }
}

#[test]
fn empty_added_line_keeps_its_tint_through_the_right_edge() {
    let change = Change::between(Some("a\n"), "a\n\n").unwrap();
    let body = render(&change, "notes.txt", 20, true);
    let blank = body
        .rows
        .iter()
        .find(|row| row.background == Some(DIFF_ADDED_SURFACE))
        .unwrap();
    assert_eq!(blank.plain(), "    2 +  ");
    assert_eq!(blank.spans[0].background, Some(Surface::TERMINAL));
    assert!(blank.paint(20).ends_with(&format!(
        "\x1b[0m\x1b[{}m{}\x1b[0m",
        DIFF_ADDED_SURFACE,
        " ".repeat(20 - text::cells(&blank.plain()))
    )));
}

#[test]
fn compact_minified_code_is_short_and_expands_without_losing_content() {
    let source = format!("{}END", "let x = 1; ".repeat(2_000));
    let change = Change::between(None, &source).unwrap();
    let compact = render(&change, "source.js", 80, false);
    assert!(compact.rows.len() <= 4);
    assert!(compact.rows[2].plain().ends_with('…'));
    let expanded = render(&change, "source.js", 80, true);
    assert!(expanded.rows.iter().any(|row| row.plain().contains("END")));
}

#[test]
fn format_only_changes_do_not_invent_changed_code() {
    for (old, new, expected) in [
        ("same", "same\n", "Added final newline"),
        ("same\r\n", "same\n", "Changed line endings"),
        ("same\n", "same\n", "No changes"),
    ] {
        let change = Change::between(Some(old), new).unwrap();
        assert_eq!(change.added + change.removed, 0);
        let text = format!(
            "{}\n{}",
            summary(&change),
            render(&change, "notes.txt", 80, false)
                .rows
                .iter()
                .map(Line::plain)
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(text.contains(expected));
    }
}

#[test]
fn changed_fragments_keep_whole_unicode_glyphs_and_insertion_boundaries() {
    for (before, after, removed, added) in [
        ("cafe\u{301}", "cafe\u{308}", "e\u{301}", "e\u{308}"),
        ("👩‍💻", "👩‍🔬", "👩‍💻", "👩‍🔬"),
        ("🇮🇹", "🇮🇪", "🇮🇹", "🇮🇪"),
        ("東京", "東海", "京", "海"),
        ("10", "120", "", "2"),
        ("120", "10", "2", ""),
        ("a b", "a  b", "", " "),
    ] {
        let old = format!("let value = \"{before}\";\n");
        let new = format!("let value = \"{after}\";\n");
        let change = Change::between(Some(&old), &new).unwrap();
        for width in [20, 40, 80] {
            let body = render(&change, "source.rs", width, true);
            assert_eq!(highlighted(&body.rows, DIFF_REMOVED_STRONG), removed);
            assert_eq!(highlighted(&body.rows, DIFF_ADDED_STRONG), added);
            assert!(body.rows.iter().all(|row| row.background.is_some()));
            assert!(
                body.rows
                    .iter()
                    .all(|row| text::cells(&row.plain()) <= width)
            );
        }
    }
}

#[test]
fn unmatched_lines_keep_syntax_and_tint_without_bold_fragments() {
    let old = "let first = 10;\nlet second = 20;\nlet gone = 30;\n";
    let new = "let first = 11;\nlet second = 21;\n";
    let change = Change::between(Some(old), new).unwrap();
    let body = render(&change, "source.rs", 80, true);
    let gone: Vec<_> = body
        .rows
        .iter()
        .filter(|row| row.plain().contains("gone"))
        .cloned()
        .collect();
    assert_eq!(gone[0].background, Some(DIFF_REMOVED_SURFACE));
    assert_eq!(highlighted(&gone, DIFF_REMOVED_STRONG), "");
    assert!(
        gone[0]
            .spans
            .iter()
            .any(|span| span.text == "let" && span.style == crate::tui::theme::KEYWORD)
    );
    assert_eq!(gone[0].spans[0].background, Some(Surface::TERMINAL));
    assert!(gone[0].spans[1].background.is_none());
    let created = render(
        &Change::between(None, "let ready = true;\n").unwrap(),
        "source.rs",
        80,
        true,
    );
    assert_eq!(highlighted(&created.rows, DIFF_ADDED_STRONG), "");
    assert!(
        created
            .rows
            .iter()
            .all(|row| row.background == Some(DIFF_ADDED_SURFACE))
    );
}

#[test]
fn fallback_snippets_tint_rows_without_inventing_coordinates() {
    let mut change = Change::between(Some("let value = 3;\n"), "let value = 12;\n").unwrap();
    for entry in change.hunks.iter_mut().flatten() {
        entry.old_line = None;
        entry.new_line = None;
    }
    let body = render_snippet(&change, "source.rs", 40, true);
    let removed = body
        .rows
        .iter()
        .find(|row| row.background == Some(DIFF_REMOVED_SURFACE))
        .unwrap();
    let added = body
        .rows
        .iter()
        .find(|row| row.background == Some(DIFF_ADDED_SURFACE))
        .unwrap();
    assert!(removed.plain().starts_with("     −  "));
    assert!(added.plain().starts_with("     +  "));
    assert_eq!(highlighted(&body.rows, DIFF_REMOVED_STRONG), "3");
    assert_eq!(highlighted(&body.rows, DIFF_ADDED_STRONG), "12");
    assert!(
        !body
            .rows
            .iter()
            .any(|row| row.plain().contains("Original file coordinates"))
    );
}

#[test]
fn long_paired_lines_keep_their_shared_tail_and_exact_changed_fragment() {
    let prefix = "界x".repeat(4_000);
    let before = format!("{prefix}OLD unchanged tail\n");
    let after = format!("{prefix}NEW unchanged tail\n");
    let change = Change::between(Some(&before), &after).unwrap();
    let compact = render(&change, "source.txt", 40, false);
    assert!(compact.rows.len() <= 6);
    let full = render(&change, "source.txt", 40, true);
    assert_eq!(highlighted(&full.rows, DIFF_REMOVED_STRONG), "OLD");
    assert_eq!(highlighted(&full.rows, DIFF_ADDED_STRONG), "NEW");
    for (source, expected) in [(2, before.trim_end()), (3, after.trim_end())] {
        let retained: String = full
            .rows
            .iter()
            .filter(|row| row.origin.is_some_and(|origin| origin.line == source))
            .flat_map(|row| row.spans.iter().skip(2))
            .map(|span| span.text.as_str())
            .collect();
        assert_eq!(retained, expected);
    }
    assert!(full.rows.iter().all(|row| text::cells(&row.plain()) <= 40));
}
