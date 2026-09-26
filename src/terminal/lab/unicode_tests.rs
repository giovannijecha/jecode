use super::*;

fn split(text: &str) -> Vec<&str> {
    clusters(text).collect()
}

#[test]
fn table_is_sorted_and_disjoint() {
    for pair in WIDE.windows(2) {
        assert!(pair[0].0 <= pair[0].1 && pair[0].1 < pair[1].0, "{pair:x?}");
    }
}

#[test]
fn accented_letters_are_one_cell_composed_or_not() {
    assert_eq!(split("é"), ["é"]);
    assert_eq!(split("e\u{301}x"), ["e\u{301}", "x"]);
    assert_eq!(width("città perché e\u{301}"), 14);
}

#[test]
fn emoji_sequences_stay_whole_and_wide() {
    for emoji in [
        "👍🏽",
        "👨\u{200d}👩\u{200d}👧",
        "❤\u{fe0f}",
        "1\u{fe0f}\u{20e3}",
        "🇮🇹",
        "⚡",
    ] {
        assert_eq!(split(emoji), [emoji], "{emoji}");
        assert_eq!(width(emoji), 2, "{emoji}");
    }
}

#[test]
fn flags_pair_left_to_right() {
    assert_eq!(split("🇮🇹🇫🇷"), ["🇮🇹", "🇫🇷"]);
    assert_eq!(split("🇮🇹🇫"), ["🇮🇹", "🇫"]);
}

#[test]
fn cjk_and_hangul_are_wide() {
    assert_eq!(width("日本語"), 6);
    assert_eq!(width("한"), 2);
    assert_eq!(
        split("\u{1112}\u{1161}\u{11ab}"),
        ["\u{1112}\u{1161}\u{11ab}"]
    );
    assert_eq!(width("\u{1112}\u{1161}\u{11ab}"), 2);
}

#[test]
fn ui_symbols_and_arrows_are_narrow() {
    assert_eq!(width("↑↓←→─│├└✓✗›·…⣿"), 14);
}

#[test]
fn text_presentation_selector_narrows() {
    assert_eq!(width("❤\u{fe0e}"), 1);
}

#[test]
fn crlf_is_one_cluster_and_controls_split() {
    assert_eq!(split("a\r\nb"), ["a", "\r\n", "b"]);
    assert_eq!(split("\u{1b}a"), ["\u{1b}", "a"]);
}
