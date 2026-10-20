use super::{Change, Kind};

#[test]
fn edit_counts_only_changed_lines_and_keeps_three_context_lines() {
    let old = "a\nb\nc\nold\ne\nf\ng\n";
    let new = "a\nb\nc\nnew\nextra\ne\nf\ng\n";
    let change = Change::between(Some(old), new).unwrap();
    assert_eq!((change.added, change.removed), (2, 1));
    assert!(!change.created);
    assert_eq!(change.hunks.len(), 1);
    let hunk = &change.hunks[0];
    assert_eq!(hunk.len(), 9);
    assert_eq!((hunk[0].old_line, hunk[0].new_line), (Some(1), Some(1)));
    assert_eq!(
        (hunk[3].kind, hunk[3].old_line, hunk[3].new_line),
        (Kind::Removed, Some(4), None)
    );
    assert_eq!(
        (hunk[5].kind, hunk[5].old_line, hunk[5].new_line),
        (Kind::Added, None, Some(5))
    );
    assert_eq!((hunk[8].old_line, hunk[8].new_line), (Some(7), Some(8)));
}

#[test]
fn distant_changes_form_separate_hunks_with_original_coordinates() {
    let old = (1..=30).map(|n| format!("line {n}\n")).collect::<String>();
    let new = old
        .replace("line 5\n", "changed 5\n")
        .replace("line 25\n", "changed 25\n");
    let change = Change::between(Some(&old), &new).unwrap();
    assert_eq!(
        (change.added, change.removed, change.hunks.len()),
        (2, 2, 2)
    );
    assert_eq!(change.hunks[0].first().unwrap().old_line, Some(2));
    assert_eq!(change.hunks[0].last().unwrap().old_line, Some(8));
    assert_eq!(change.hunks[1].first().unwrap().old_line, Some(22));
    assert_eq!(change.hunks[1].last().unwrap().old_line, Some(28));
}

#[test]
fn insertion_and_removal_shift_new_coordinates() {
    let change = Change::between(Some("a\nb\nc\nd\n"), "a\nx\nb\nd\n").unwrap();
    assert_eq!((change.added, change.removed), (1, 1));
    let entries = change.hunks.into_iter().flatten().collect::<Vec<_>>();
    assert!(
        entries
            .iter()
            .any(|entry| entry.kind == Kind::Added && entry.new_line == Some(2))
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.kind == Kind::Removed && entry.old_line == Some(3))
    );
    assert!(
        entries.iter().any(|entry| entry.text == "d"
            && entry.old_line == Some(4)
            && entry.new_line == Some(4))
    );
}

#[test]
fn empty_creation_and_noop_have_no_invented_rows() {
    let created = Change::between(None, "").unwrap();
    assert!(created.created);
    assert_eq!((created.added, created.removed), (0, 0));
    assert!(created.hunks.is_empty());
    let unchanged = Change::between(Some("a\n"), "a\n").unwrap();
    assert!(!unchanged.created);
    assert_eq!((unchanged.added, unchanged.removed), (0, 0));
    assert!(unchanged.hunks.is_empty());
}

#[test]
fn final_newline_and_crlf_changes_do_not_inflate_content_counts() {
    let newline = Change::between(Some("one"), "one\n").unwrap();
    assert_eq!((newline.added, newline.removed), (0, 0));
    assert_eq!(newline.final_newline, Some(true));
    assert!(newline.hunks.is_empty());
    let endings = Change::between(Some("one\r\ntwo\r\n"), "one\ntwo\n").unwrap();
    assert_eq!((endings.added, endings.removed), (0, 0));
    assert_eq!(endings.ending_change.as_deref(), Some("CRLF → LF"));
    assert!(endings.hunks.is_empty());
    let mixed = Change::between(Some("one\r\ntwo\nthree\r\n"), "one\ntwo\r\nthree\r\n").unwrap();
    assert_eq!((mixed.added, mixed.removed), (0, 0));
    assert_eq!(
        mixed.ending_change.as_deref(),
        Some("mixed line endings updated")
    );
    assert!(mixed.hunks.is_empty());
    let content = Change::between(Some("one\r\ntwo"), "one\r\nthree\r\n").unwrap();
    assert_eq!((content.added, content.removed), (1, 1));
    assert_eq!(content.final_newline, Some(true));
    assert!(
        content.hunks[0]
            .iter()
            .any(|entry| entry.text == "two" && entry.no_newline)
    );
    let context = Change::between(Some("before\nlast"), "after\nlast\n").unwrap();
    assert_eq!(context.final_newline, Some(true));
    assert!(
        context.hunks[0]
            .iter()
            .any(|entry| entry.kind == Kind::Context && entry.text == "last" && !entry.no_newline)
    );
}

#[test]
fn persisted_value_round_trips() {
    let change = Change::between(Some("a\nb\nc\n"), "a\nx\nc").unwrap();
    assert_eq!(Change::from_value(&change.value()).unwrap(), change);
}

#[test]
fn mixed_line_endings_are_detected_across_insertions() {
    let change = Change::between(Some("a\r\nb\n"), "x\na\nb\r\n").unwrap();
    assert_eq!((change.added, change.removed), (1, 0));
    assert_eq!(
        change.ending_change.as_deref(),
        Some("mixed line endings updated")
    );
}

#[test]
fn optional_capture_bounds_bytes_lines_and_alignment_work() {
    assert!(Change::between(None, &"x".repeat(super::MAX_DIFF_BYTES + 1)).is_none());
    assert!(Change::between(None, &"\n".repeat(super::MAX_DIFF_LINES + 1)).is_none());
    let previous = format!("{}shared\n", "old\n".repeat(19_999));
    let updated = format!("shared\n{}", "new\n".repeat(19_999));
    assert!(Change::between(Some(&previous), &updated).is_none());
    // Large disjoint rewrites need no alignment search and remain cheap.
    let change = Change::between(Some(&"old\n".repeat(10_000)), &"new\n".repeat(10_000)).unwrap();
    assert_eq!((change.added, change.removed), (10_000, 10_000));
}

#[test]
fn repeated_lines_still_produce_valid_alignment() {
    let old = "same\nsame\na\nsame\n";
    let new = "same\na\nsame\nsame\n";
    let change = Change::between(Some(old), new).unwrap();
    assert_eq!((change.added, change.removed), (1, 1));
    for hunk in &change.hunks {
        for entry in hunk {
            if let Some(line) = entry.old_line {
                assert_eq!(old.lines().nth(line - 1), Some(entry.text.as_str()));
            }
            if let Some(line) = entry.new_line {
                assert_eq!(new.lines().nth(line - 1), Some(entry.text.as_str()));
            }
        }
    }
}
