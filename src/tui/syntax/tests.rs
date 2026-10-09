use super::*;

#[test]
fn rust_keywords_numbers_strings_and_comments_keep_the_original_code() {
    let source = r#"let widget = 40; println!("fn 40"); // let"#;
    let row = Highlighter::new("rust").line(source);
    assert_eq!(row.plain(), source);
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "let" && span.style == KEYWORD)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "40" && span.style == CODE_NUMBER)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "\"fn 40\"" && span.style == CODE_STRING)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "// let" && span.style == MUTED)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text.contains("widget") && span.style == CODE_TEXT)
    );
}

#[test]
fn rust_lifetimes_are_distinct_from_unicode_and_escaped_char_literals() {
    let source = r"fn take<'a>(x: &'a str) -> char { 'é'; '\n'; '\u{03b1}'; '\x41' }";
    let row = Highlighter::new("rust").line(source);
    assert_eq!(row.plain(), source);
    let strings: Vec<_> = row
        .spans
        .iter()
        .filter(|span| span.style == CODE_STRING)
        .map(|span| span.text.as_str())
        .collect();
    assert_eq!(strings, ["'é'", r"'\n'", r"'\u{03b1}'", r"'\x41'"]);
}

#[test]
fn nested_comments_and_raw_strings_continue_across_lines_and_end_cleanly() {
    let mut highlighter = Highlighter::new("rs");
    let sources = [
        "/* outer",
        "/* inner */ fn 40",
        "*/ let x = r#\"fn",
        "40\"#; let y = 2;",
    ];
    let rows: Vec<_> = sources
        .iter()
        .map(|source| highlighter.line(source))
        .collect();
    for (row, source) in rows.iter().zip(sources) {
        assert_eq!(row.plain(), source);
    }
    assert!(rows[1].spans.iter().all(|span| span.style == MUTED));
    assert!(
        rows[2]
            .spans
            .iter()
            .any(|span| span.text == "let" && span.style == KEYWORD)
    );
    assert!(
        rows[2]
            .spans
            .iter()
            .any(|span| span.text == "r#\"fn" && span.style == CODE_STRING)
    );
    assert!(
        rows[3]
            .spans
            .iter()
            .any(|span| span.text == "40\"#" && span.style == CODE_STRING)
    );
    assert!(
        rows[3]
            .spans
            .iter()
            .any(|span| span.text == "2" && span.style == CODE_NUMBER)
    );
}

#[test]
fn shell_and_json_literals_are_highlighted_without_interpreting_their_contents() {
    let shell = "if printf '%s' 'é40'; then # 40";
    let row = Highlighter::new("bash").line(shell);
    assert_eq!(row.plain(), shell);
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "if" && span.style == KEYWORD)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "'é40'" && span.style == CODE_STRING)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "# 40" && span.style == MUTED)
    );
    let json = r#"{"escaped": "\"true\"", "enabled": true, "n": 40}"#;
    let row = Highlighter::new("JSON").line(json);
    assert_eq!(row.plain(), json);
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "true" && span.style == KEYWORD)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "40" && span.style == CODE_NUMBER)
    );
    let mut shell = Highlighter::new("sh");
    assert_eq!(shell.line("echo 'first").plain(), "echo 'first");
    let row = shell.line("if 40' ; then");
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "if 40'" && span.style == CODE_STRING)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "then" && span.style == KEYWORD)
    );
}

#[test]
fn unlabelled_and_unsupported_languages_stay_readable_plain_text() {
    for label in ["", "python", "unavailable"] {
        let source = "  **text** `literal` λ40";
        let row = Highlighter::new(label).line(source);
        assert_eq!(row.plain(), source);
        assert!(row.spans.iter().all(|span| span.style == CODE_TEXT));
    }
}
