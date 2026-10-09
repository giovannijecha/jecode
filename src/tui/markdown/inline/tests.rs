use super::*;
use crate::tui::theme::{BODY, INLINE_CODE};

fn spans(value: &str) -> Vec<(String, &'static str)> {
    render(value, BODY)
        .spans
        .into_iter()
        .map(|span| (span.text, span.style))
        .collect()
}

#[test]
fn common_emphasis_and_strike_combine_without_dropping_text() {
    assert_eq!(
        spans("A *soft* _quiet_ **loud** __firm__ ~~old~~ ***both***."),
        vec![
            ("A ".into(), BODY),
            ("soft".into(), ITALIC),
            (" ".into(), BODY),
            ("quiet".into(), ITALIC),
            (" ".into(), BODY),
            ("loud".into(), EMPHASIS),
            (" ".into(), BODY),
            ("firm".into(), EMPHASIS),
            (" ".into(), BODY),
            ("old".into(), STRIKE),
            (" ".into(), BODY),
            ("both".into(), ITALIC_STRONG),
            (".".into(), BODY),
        ]
    );
    assert_eq!(
        spans("**bold _and italic_** / ~~gone *softly*~~"),
        vec![
            ("bold ".into(), EMPHASIS),
            ("and italic".into(), ITALIC_STRONG),
            (" / ".into(), BODY),
            ("gone ".into(), STRIKE),
            ("softly".into(), ITALIC_STRIKE),
        ]
    );
}

#[test]
fn code_spans_override_markers_and_escapes_protect_punctuation() {
    assert_eq!(
        spans(
            "**bold \\*star\\* and \\\u{0060}tick\\\u{0060}** / \u{0060}*raw* \\_ \\[link](x)\u{0060}"
        ),
        vec![
            ("bold *star* and \u{0060}tick\u{0060}".into(), EMPHASIS),
            (" / ".into(), BODY),
            ("*raw* \\_ \\[link](x)".into(), INLINE_CODE),
        ]
    );
    assert_eq!(render(r"\*x\* \_y\_ \~~z~~", BODY).plain(), "*x* _y_ ~~z~~");
}

#[test]
fn links_keep_destinations_visible_and_style_labels() {
    assert_eq!(
        spans("See [the *notes*](https://example.test/a_(b)) today."),
        vec![
            ("See ".into(), BODY),
            ("the ".into(), LINK),
            ("notes".into(), LINK_ITALIC),
            (" (".into(), BODY),
            ("https://example.test/a_(b)".into(), LINK_DESTINATION),
            (") today.".into(), BODY),
        ]
    );
    assert_eq!(
        render("![logo](image.png) [broken]( ) [open](url", BODY).plain(),
        "![logo](image.png) [broken]( ) [open](url"
    );
}

#[test]
fn underscores_inside_words_and_literal_paths_stay_visible() {
    let source = "snake_case /tmp/a_b C:\\Users\\a_b file_name.rs a__b";
    assert_eq!(render(source, BODY).plain(), source);
    assert_eq!(render("_edge_ __clear__", BODY).plain(), "edge clear");
    assert_eq!(
        render("*unclosed ~~still", BODY).plain(),
        "*unclosed ~~still"
    );
    assert_eq!(
        render("literal **** and ** **", BODY).plain(),
        "literal **** and ** **"
    );
}

#[test]
fn deeply_nested_input_has_a_visible_literal_fallback() {
    let mut source = "center".to_string();
    for _ in 0..MAX_DEPTH + 2 {
        source = format!("[{source}](x)");
    }
    let result = render(&source, BODY).plain();
    assert!(result.contains("center"));
    assert!(result.contains("[center](x)"));
}
