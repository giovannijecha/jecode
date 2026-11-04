use super::{Edges, Regex, Syntax, parser};

fn found(pattern: &str, text: &str) -> Option<(usize, usize)> {
    Regex::new(&[pattern], Syntax::Extended, Edges::Any, false)
        .unwrap()
        .find(text.as_bytes(), 0)
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

/// Compiles on a stack with room left of the 1 MiB a shell can run on.
fn compiles(pattern: String) -> Result<(), String> {
    std::thread::Builder::new()
        .stack_size(768 * 1024)
        .spawn(move || {
            Regex::new(&[&pattern], Syntax::Extended, Edges::Any, false).map(|regex| drop(regex))
        })
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn deep_nesting_is_refused_before_it_can_overflow_the_stack() {
    let groups = format!("{}a{}", "(".repeat(5000), ")".repeat(5000));
    let error = compiles(groups).unwrap_err();
    assert!(error.contains("nests too deeply"), "{error}");
    let stacked = format!("a{}", "*".repeat(100_000));
    let error = compiles(stacked).unwrap_err();
    assert!(error.contains("nests too deeply"), "{error}");
    let mut deepest = "a".to_owned();
    for _ in 0..parser::MAX_GROUPS {
        deepest = format!("({deepest}{})", "{1}".repeat(parser::MAX_STACKED));
    }
    assert_eq!(compiles(deepest), Ok(()));
}

#[test]
fn word_start_and_word_end_have_distinct_directions() {
    assert_eq!(found(r"\<a", "a"), Some((0, 1)));
    assert_eq!(found(r"a\>", "a"), Some((0, 1)));
    assert_eq!(found(r"\>a", "a"), None);
    assert_eq!(found(r"a\<", "a"), None);
}
