use super::*;
use crate::workspace_fixture as support;
use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}

#[test]
fn replacement_must_validate_composed_text_across_staging_threshold() {
    let files = support::Fixture::new();
    let ws = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    // The replacement contributes seven bytes to the staging decision.
    // These adjacent lengths select opposite sides of that decision.
    for padding in [10, MAX_FILE_BYTES - 15, MAX_FILE_BYTES - 14, 1_100_000] {
        let source = format!("{}\r\nUNIQUE", "a".repeat(padding));
        files.write("join.txt", &source);
        let error = ws
            .prepare_edit("join.txt", "\nUNIQUE", "changed", &budget(&cancelled))
            .err()
            .expect("a replacement leaving bare CR must be rejected before approval");
        assert!(error.to_string().contains("text"));
        assert_eq!(
            fs::read(files.0.join("join.txt")).unwrap(),
            source.as_bytes()
        );
        assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
    }
}

#[test]
fn replacement_leaving_bare_cr_at_eof_is_rejected_without_artifacts() {
    for padding in [10, 1_100_000] {
        let files = support::Fixture::new();
        let ws = Workspace::open(&files.0).unwrap();
        let cancelled = AtomicBool::new(false);
        let source = format!("{}\r\n", "a".repeat(padding));
        files.write("join.txt", &source);
        assert!(
            ws.prepare_edit("join.txt", "\n", "", &budget(&cancelled))
                .is_err()
        );
        assert_eq!(
            fs::read(files.0.join("join.txt")).unwrap(),
            source.as_bytes()
        );
        assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
    }
}

#[test]
fn valid_join_replacements_preserve_bytes_and_recovery_on_both_paths() {
    for padding in [10, 1_100_000] {
        for (old, new) in [
            ("\nUNIQUE", "\nchanged"),
            ("\r\nUNIQUE", "\nchanged"),
            ("\r\nUNIQUE", ""),
            ("UNIQUE", "✨\t"),
        ] {
            let files = support::Fixture::new();
            let ws = Workspace::open(&files.0).unwrap();
            let cancelled = AtomicBool::new(false);
            let source = format!("{}\r\nUNIQUE\t世界", "a".repeat(padding));
            files.write("join.txt", &source);
            let change = ws
                .prepare_edit("join.txt", old, new, &budget(&cancelled))
                .unwrap();
            let staged = matches!(
                change.before.as_ref().unwrap().original,
                Original::Staged(_)
            );
            assert_eq!(staged, padding > MAX_FILE_BYTES);
            let applied = ws.apply(change, &budget(&cancelled)).unwrap();
            assert_eq!(
                fs::read(files.0.join("join.txt")).unwrap(),
                source.replace(old, new).as_bytes()
            );
            assert_eq!(
                fs::read(files.0.join(applied.recovery.unwrap())).unwrap(),
                source.as_bytes()
            );
        }
    }
}

#[test]
fn original_401_line_create_is_accepted_with_bounded_inspectable_preview() {
    let files = support::Fixture::new();
    let ws = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let content: String = (0..401).map(|n| format!("line {n:03} abc\n")).collect();
    assert_eq!(content.len(), 5213);
    let change = ws
        .prepare_create("page.html", &content, &budget(&cancelled))
        .unwrap();
    let preview = change.preview();
    assert_eq!((preview.added, preview.removed), (401, 0));
    assert!(preview.diff.lines().count() <= 400);
    assert!(preview.diff.len() <= 48 * 1024);
    assert_eq!(preview.omitted_lines, 2); // one source line plus its diff prefix accounting
    let artifact = preview.full_diff_path.clone().unwrap();
    let full = fs::read_to_string(&artifact).unwrap();
    assert!(full.contains("+ line 000 abc"));
    assert!(full.contains("+ line 400 abc"));
    assert_eq!(full.len() - preview.diff.len(), preview.omitted_bytes);
    assert!(fs::read_to_string(files.0.join("page.html")).is_err());
    // A writable informational copy must never supply publication bytes.
    let _ = fs::write(&artifact, "tampered display copy");
    ws.apply(change, &budget(&cancelled)).unwrap();
    assert_eq!(
        fs::read(files.0.join("page.html")).unwrap(),
        content.as_bytes()
    );
    assert!(!std::path::Path::new(&artifact).exists());
}

#[test]
fn formatted_large_create_and_replacement_preserve_exact_bytes_and_recovery() {
    let files = support::Fixture::new();
    let ws = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let source: String = (0..5000)
        .map(|n| format!("    <section id=\"node-{n:05}\">Hello</section>\n"))
        .collect();
    assert!((64 * 1024..=256 * 1024).contains(&source.len()));
    let started = Instant::now();
    let change = ws
        .prepare_create("formatted.html", &source, &budget(&cancelled))
        .unwrap();
    eprintln!(
        "formatted create: {} input bytes, {} preview bytes, {} ms prepare+preview",
        source.len(),
        change.preview().diff.len(),
        started.elapsed().as_millis()
    );
    assert!(change.preview().omitted_lines > 0);
    let full = fs::read_to_string(change.preview().full_diff_path.as_ref().unwrap()).unwrap();
    assert!(full.contains("node-04999"));
    assert!(started.elapsed() < Duration::from_secs(8));
    ws.apply(change, &budget(&cancelled)).unwrap();
    assert_eq!(
        fs::read(files.0.join("formatted.html")).unwrap(),
        source.as_bytes()
    );

    let old = format!("{}\n", "old-text".repeat(5500));
    let new = format!("{}\n", "new-text".repeat(6200));
    assert!(old.len() + new.len() > 32 * 1024);
    files.write("replace.txt", &old);
    let change = ws
        .prepare_edit("replace.txt", &old, &new, &budget(&cancelled))
        .unwrap();
    assert!(change.preview().omitted_bytes > 0);
    let full = fs::read_to_string(change.preview().full_diff_path.as_ref().unwrap()).unwrap();
    assert!(full.contains("new-text"));
    let applied = ws.apply(change, &budget(&cancelled)).unwrap();
    assert_eq!(
        fs::read(files.0.join("replace.txt")).unwrap(),
        new.as_bytes()
    );
    assert_eq!(
        fs::read(files.0.join(applied.recovery.unwrap())).unwrap(),
        old.as_bytes()
    );
}

#[test]
fn staged_large_edit_and_paginated_read_preserve_unicode_crlf_tabs_and_no_final_newline() {
    let files = support::Fixture::new();
    let ws = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let mut source = "    before\r\n".repeat(100_000);
    source.push_str("\tα UNIQUE target\r\n");
    source.push_str(&"    after\r\n".repeat(4));
    source.push_str("last line without newline");
    assert!(source.len() > 1024 * 1024);
    files.write("large.txt", &source);
    let page = ws
        .read_page("large.txt", 100_001, 2, &budget(&cancelled))
        .unwrap();
    assert!(page.text.starts_with("\tα UNIQUE target\r\n"));
    assert!(page.truncated);
    let started = Instant::now();
    let change = ws
        .prepare_edit(
            "large.txt",
            "UNIQUE target",
            "changed\t世界",
            &budget(&cancelled),
        )
        .unwrap();
    eprintln!(
        "staged edit: {} original bytes, {} preview bytes, {} ms prepare+preview",
        source.len(),
        change.preview().diff.len(),
        started.elapsed().as_millis()
    );
    assert!(matches!(
        change.before.as_ref().unwrap().original,
        Original::Staged(_)
    ));
    assert!(matches!(change.after, After::Replacement { .. }));
    assert!(started.elapsed() < Duration::from_secs(8));
    let preview = change.preview();
    assert_eq!((preview.added, preview.removed), (1, 1));
    let applied = ws.apply(change, &budget(&cancelled)).unwrap();
    let expected = source.replace("UNIQUE target", "changed\t世界");
    assert_eq!(
        fs::read(files.0.join("large.txt")).unwrap(),
        expected.as_bytes()
    );
    assert_eq!(
        fs::read(files.0.join(applied.recovery.unwrap())).unwrap(),
        source.as_bytes()
    );
}

#[test]
fn staged_large_deletion_has_full_diff_and_stale_or_cancelled_changes_do_not_publish() {
    let files = support::Fixture::new();
    let ws = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let deletion = "    remove this line\r\n".repeat(3000);
    let source = format!("{}{}TAIL", "prefix\n".repeat(160_000), deletion);
    files.write("large.txt", &source);
    let change = ws
        .prepare_edit("large.txt", &deletion, "", &budget(&cancelled))
        .unwrap();
    assert!(change.preview().omitted_lines > 0);
    assert_eq!(change.preview().removed, 3000);
    let artifact = change.preview().full_diff_path.clone().unwrap();
    assert!(
        fs::read_to_string(&artifact)
            .unwrap()
            .contains("remove this line")
    );
    cancelled.store(true, Ordering::Release);
    assert!(ws.apply(change, &budget(&cancelled)).is_err());
    assert_eq!(
        fs::read(files.0.join("large.txt")).unwrap(),
        source.as_bytes()
    );
    assert!(!std::path::Path::new(&artifact).exists());
    cancelled.store(false, Ordering::Release);
    let change = ws
        .prepare_edit("large.txt", &deletion, "", &budget(&cancelled))
        .unwrap();
    files.write("large.txt", &(source.clone() + "changed"));
    assert!(
        ws.apply(change, &budget(&cancelled))
            .err()
            .unwrap()
            .to_string()
            .contains("changed since")
    );
    assert_eq!(
        fs::read_to_string(files.0.join("large.txt")).unwrap(),
        source + "changed"
    );
}

#[test]
fn competing_create_and_ambiguous_large_edit_leave_targets_untouched() {
    let files = support::Fixture::new();
    let ws = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let content = "line\n".repeat(401);
    let change = ws
        .prepare_create("new.txt", &content, &budget(&cancelled))
        .unwrap();
    files.write("new.txt", "competing\n");
    assert!(
        ws.apply(change, &budget(&cancelled))
            .err()
            .unwrap()
            .to_string()
            .contains("appeared")
    );
    assert_eq!(
        fs::read_to_string(files.0.join("new.txt")).unwrap(),
        "competing\n"
    );

    let source = format!(
        "{}token\n{}token\n",
        "prefix\n".repeat(160_000),
        "middle\n".repeat(2)
    );
    files.write("large.txt", &source);
    let error = ws
        .prepare_edit("large.txt", "token", "replacement", &budget(&cancelled))
        .err()
        .unwrap();
    assert!(error.to_string().contains("ambiguous"));
    assert_eq!(
        fs::read(files.0.join("large.txt")).unwrap(),
        source.as_bytes()
    );
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 2);
}
