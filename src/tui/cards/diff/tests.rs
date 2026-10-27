use super::*;
use crate::tui::theme::{
    BASE, DIFF_ADDED_STRONG, DIFF_ADDED_SURFACE, DIFF_REMOVED_STRONG, DIFF_REMOVED_SURFACE, Surface,
};

/// The code on a diff row: everything after the `  ` gutter, the number
/// (`digits + 2` cells), a space and the mark with its space.
fn code(row: &Line, digits: usize) -> String {
    row.plain().chars().skip(2 + digits + 2 + 1 + 2).collect()
}

/// Wrapped rows keep their source: each row's code, past the hang, is the
/// source from the row's origin, and only whitespace falls between rows.
fn assert_retained(rows: &[&Line], source: &str, digits: usize) {
    let mut retained = String::new();
    for (part, row) in rows.iter().enumerate() {
        let code = code(row, digits);
        let code = if part == 0 {
            code.as_str()
        } else {
            code.trim_start()
        };
        let from: String = source.chars().skip(row.origin.unwrap().character).collect();
        assert!(from.starts_with(code), "{code:?} is not at its origin");
        retained.push_str(code);
    }
    let visible = |text: &str| text.split_whitespace().collect::<String>();
    assert_eq!(visible(&retained), visible(source));
}

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
            "    1   fn main() {",
            "    2 −     let limit = 3;",
            "    2 +     let limit = 12;",
            "    3   }"
        ]
    );
    assert_eq!(body.rows[1].spans[2].style, BAD);
    assert_eq!(body.rows[2].spans[2].style, GOOD);
    assert_eq!(body.rows[0].background, None);
    assert_eq!(body.rows[1].background, Some(DIFF_REMOVED_SURFACE));
    assert_eq!(body.rows[2].background, Some(DIFF_ADDED_SURFACE));
    assert_eq!(body.rows[3].background, None);
    assert_eq!(highlighted(&body.rows, DIFF_REMOVED_STRONG), "3");
    assert_eq!(highlighted(&body.rows, DIFF_ADDED_STRONG), "12");
    // Context keeps its coordinates quiet and its code in the code colour.
    assert_eq!(body.rows[0].spans[1].style, DIM);
    assert!(
        body.rows[0]
            .spans
            .iter()
            .skip(2)
            .all(|span| span.style == CODE_TEXT)
    );
    assert!(
        body.rows[2]
            .spans
            .iter()
            .any(|span| span.text == "let" && span.style == crate::tui::theme::KEYWORD)
    );
    let painted = body.rows[2].paint(80);
    let (margin, rest) = painted.split_once("    ").unwrap();
    assert!(margin.contains(&BASE.to_string()));
    assert!(rest.contains(&DIFF_ADDED_SURFACE.to_string()));
    assert!(rest.contains(&DIFF_ADDED_STRONG.to_string()));
    assert!(painted.ends_with(&format!(
        "\x1b[0m\x1b[{};{}m{}\x1b[0m",
        crate::tui::theme::BODY,
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
    assert!(separator.spans.iter().all(|span| span.style == DIM));
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
        assert_retained(&rows, source, 1);
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= width));
        for row in rows.iter().skip(1) {
            assert!(row.spans[1].text.chars().all(|ch| ch == ' '));
            assert!(row.plain().chars().take(8).all(|ch| ch == ' '));
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
    assert_eq!(blank.plain(), "    2 + ");
    assert_eq!(blank.spans[0].background, Some(BASE));
    assert!(blank.paint(20).ends_with(&format!(
        "\x1b[0m\x1b[{};{}m{}\x1b[0m",
        crate::tui::theme::BODY,
        DIFF_ADDED_SURFACE,
        " ".repeat(20 - text::cells(&blank.plain()))
    )));
}

#[test]
fn compact_minified_code_is_short_and_expands_without_losing_content() {
    let source = format!("{}END", "let x = 1; ".repeat(2_000));
    let change = Change::between(None, &source).unwrap();
    let compact = render(&change, "source.js", 80, false);
    // A changed line keeps two rows, the last cut with `…`, then the note.
    assert_eq!(compact.rows.len(), 3);
    assert!(
        compact.rows[1].plain().ends_with("…"),
        "{}",
        compact.rows[1].plain()
    );
    assert!(compact.rows[2].plain().contains("No newline"));
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
        // Fragments snap to whole words, runs of spaces and glyphs.
        ("cafe\u{301}", "cafe\u{308}", "cafe\u{301}", "cafe\u{308}"),
        ("👩‍💻", "👩‍🔬", "👩‍💻", "👩‍🔬"),
        ("🇮🇹", "🇮🇪", "🇮🇹", "🇮🇪"),
        ("東京", "東海", "東京", "東海"),
        ("東 京", "東 海", "京", "海"),
        ("10", "120", "10", "120"),
        ("1 0", "1 2 0", "", "2 "),
        ("a b", "a  b", " ", "  "),
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
    assert_eq!(gone[0].spans[0].background, Some(BASE));
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
    assert!(
        removed.plain().starts_with("     − let"),
        "{}",
        removed.plain()
    );
    assert!(added.plain().starts_with("     + let"), "{}", added.plain());
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
    // A space ends the shared word, so the fragment is the changed word alone.
    let before = format!("{prefix} OLD unchanged tail\n");
    let after = format!("{prefix} NEW unchanged tail\n");
    let change = Change::between(Some(&before), &after).unwrap();
    let compact = render(&change, "source.txt", 40, false);
    assert!(compact.rows.len() <= 6);
    // Collapsed, both rows slide to the fragment and keep their widths.
    assert_eq!(highlighted(&compact.rows, DIFF_REMOVED_STRONG), "OLD");
    assert_eq!(highlighted(&compact.rows, DIFF_ADDED_STRONG), "NEW");
    assert!(
        compact
            .rows
            .iter()
            .all(|row| text::cells(&row.plain()) <= 40)
    );
    let full = render(&change, "source.txt", 40, true);
    assert_eq!(highlighted(&full.rows, DIFF_REMOVED_STRONG), "OLD");
    assert_eq!(highlighted(&full.rows, DIFF_ADDED_STRONG), "NEW");
    for (source, expected) in [(2, before.trim_end()), (3, after.trim_end())] {
        let rows: Vec<_> = full
            .rows
            .iter()
            .filter(|row| row.origin.is_some_and(|origin| origin.line == source))
            .collect();
        assert_retained(&rows, expected, 1);
    }
    assert!(full.rows.iter().all(|row| text::cells(&row.plain()) <= 40));
}

#[test]
fn collapsed_minified_pairs_slide_to_their_change_with_aligned_windows() {
    let rules = "@media(max-width:600px){.hero{height:520px}.header{padding:20px 18px}.brand{font-size:28px}.hero h1{top:32%;font-size:20vw}.section{padding:48px 18px}.card-grid{grid-template-columns:1fr";
    let context = format!("{}\n", "html{scroll-behavior:smooth}".repeat(8));
    let before = format!("{context}{rules}}}}}\n");
    let after = format!("{context}{rules};gap:8px}}}}\n");
    let change = Change::between(Some(&before), &after).unwrap();
    for columns in [60, 80, 100] {
        let body = render(&change, "style.css", columns, false);
        assert!(
            body.rows
                .iter()
                .all(|row| text::cells(&row.plain()) <= columns)
        );
        // Context keeps one row, cut with `…`.
        assert_eq!(body.rows[0].background, None);
        assert!(body.rows[0].plain().ends_with('…'));
        assert_ne!(body.rows[1].background, None);
        for surface in [DIFF_REMOVED_SURFACE, DIFF_ADDED_SURFACE] {
            let rows = body
                .rows
                .iter()
                .filter(|row| row.background == Some(surface));
            assert!(rows.count() <= 2);
        }
        // The whole fragment is visible on the collapsed rows.
        assert_eq!(highlighted(&body.rows, DIFF_ADDED_STRONG), ";gap:8px");
        let first = |surface| {
            let row = body.rows.iter().find(|row| row.background == Some(surface));
            code(row.unwrap(), 1)
        };
        let (removed, added) = (first(DIFF_REMOVED_SURFACE), first(DIFF_ADDED_SURFACE));
        assert!(
            removed.starts_with('…') && added.starts_with('…'),
            "{removed:?}"
        );
        let shared = removed
            .chars()
            .zip(added.chars())
            .take_while(|(a, b)| a == b);
        assert!(shared.count() >= 12, "{removed:?} {added:?}");
        let expanded = render(&change, "style.css", columns, true);
        assert!(
            expanded
                .rows
                .iter()
                .all(|row| !code(row, 1).starts_with('…'))
        );
    }
}

#[test]
fn markup_fragments_snap_to_whole_tags() {
    let before = "<div><h3>CASA</h3></div><p>Set between the woods.</p></div></article>\n";
    let after = "<div><h3>CASA</h3></div></div></article>\n";
    let change = Change::between(Some(before), after).unwrap();
    for expanded in [false, true] {
        let body = render(&change, "projects.html", 80, expanded);
        let removed = highlighted(&body.rows, DIFF_REMOVED_STRONG);
        assert!(removed.starts_with("<p>Set"), "{removed:?}");
        assert!(removed.ends_with("</p>"), "{removed:?}");
        assert_eq!(highlighted(&body.rows, DIFF_ADDED_STRONG), "");
    }
}

#[test]
fn narrow_paired_windows_reach_changes_after_ascii_cjk_and_tabs() {
    for (before, after, columns, removed) in [
        ("abcdefghij OLD\n", "abcdefghij NEW\n", 15, true),
        ("東京東京東京 OLD\n", "東京東京東京 NEW\n", 15, true),
        ("abc\tOLD\n", "abc\tNEW\n", 14, true),
        ("abcdefghij \n", "abcdefghij NEW\n", 15, false),
        ("abcdefghij() \n", "abcdefghij(); \n", 15, false),
        ("abcdefghij OLD_LONG\n", "abcdefghij NEW_LONG\n", 12, true),
    ] {
        let change = Change::between(Some(before), after).unwrap();
        let body = render(&change, "source.txt", columns, false);
        let added = highlighted(&body.rows, DIFF_ADDED_STRONG);
        let old = highlighted(&body.rows, DIFF_REMOVED_STRONG);
        assert!(
            !added.is_empty(),
            "{before:?} -> {after:?}: {:?}",
            body.rows.iter().map(Line::plain).collect::<Vec<_>>()
        );
        assert_eq!(!old.is_empty(), removed, "{before:?} -> {after:?}");
        assert!(
            body.rows
                .iter()
                .all(|row| text::cells(&row.plain()) <= columns)
        );
    }
}
