use super::Regex;

fn found(pattern: &str, text: &str) -> Option<(usize, usize)> {
    Regex::new(pattern, false)
        .unwrap()
        .find(&text.chars().collect::<Vec<_>>(), 0)
}

#[test]
fn nested_quantifiers_and_long_lines_use_bounded_states() {
    let text = format!("{}c", "a".repeat(16_384));
    assert_eq!(found("(a+)+b", &text), None);
    assert_eq!(found("a+", &text), Some((0, 16_384)));
    assert_eq!(found("(a*)*", "x"), Some((0, 0)));
}

#[test]
fn alternatives_and_repetition_choose_the_leftmost_longest_match() {
    assert_eq!(found("a|ab", "zab"), Some((1, 3)));
    assert_eq!(found("ab|a", "zab"), Some((1, 3)));
    assert_eq!(found("a{2,4}", "aaaaa"), Some((0, 4)));
    assert_eq!(found("(a|aa)+b", "zaaab"), Some((1, 5)));
    assert_eq!(found("a*|b", "b"), Some((0, 1)));
    assert_eq!(found("^a+$", "aaa"), Some((0, 3)));
    assert_eq!(found("^a+$", "zaaa"), None);
}

#[test]
fn word_start_and_word_end_have_distinct_directions() {
    assert_eq!(found(r"\<a", "a"), Some((0, 1)));
    assert_eq!(found(r"a\>", "a"), Some((0, 1)));
    assert_eq!(found(r"\>a", "a"), None);
    assert_eq!(found(r"a\<", "a"), None);
}
