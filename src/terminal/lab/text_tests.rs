use super::*;

#[test]
fn safe_neutralises_escapes_and_normalises_newlines() {
    assert_eq!(safe("a\u{1b}[31mb"), "a?[31mb");
    assert_eq!(safe("x\r\ny\rz"), "x\ny\nz");
    assert_eq!(safe("\u{202e}evil"), "?evil");
    assert_eq!(safe("\tx"), "    x");
}

#[test]
fn wrap_breaks_at_words_and_drops_the_seam_space() {
    assert_eq!(wrap("the quick brown fox", 10), ["the quick", "brown fox"]);
}

#[test]
fn wrap_hard_breaks_long_words_between_clusters() {
    assert_eq!(wrap("abcdefgh", 3), ["abc", "def", "gh"]);
}

#[test]
fn wrap_never_splits_wide_or_joined_clusters() {
    assert_eq!(wrap("日本語", 5), ["日本", "語"]);
    assert_eq!(
        wrap("ab👨\u{200d}👩\u{200d}👧", 3),
        ["ab", "👨\u{200d}👩\u{200d}👧"]
    );
    assert_eq!(
        wrap("e\u{301}e\u{301}e\u{301}", 2),
        ["e\u{301}e\u{301}", "e\u{301}"]
    );
}

#[test]
fn wrap_keeps_blank_lines() {
    assert_eq!(wrap("a\n\nb", 5), ["a", "", "b"]);
}

#[test]
fn clip_fits_exactly_with_ellipsis() {
    assert_eq!(clip("hello world", 8, "…"), "hello w…");
    assert_eq!(clip("日本語テキスト", 6, "…"), "日本…");
    assert_eq!(clip("short", 8, "…"), "short");
    assert_eq!(width(&clip("👍🏽👍🏽👍🏽", 5, "...")), 5);
}

#[test]
fn pad_counts_cells_not_bytes() {
    assert_eq!(width(&pad("città", 8)), 8);
    assert_eq!(width(&pad("日本", 6)), 6);
}

#[test]
fn wrap_line_ranges_exclude_seam_spaces() {
    let line = "alpha beta gamma";
    let rows: Vec<&str> = wrap_line(line, 11).into_iter().map(|r| &line[r]).collect();
    assert_eq!(rows, ["alpha beta", "gamma"]);
    let empty = wrap_line("", 5);
    assert!(empty.len() == 1 && empty[0].is_empty());
}
