use super::*;
use crate::terminal::lab::glyph::UNICODE;

fn render(source: &str, width: usize) -> Vec<Row> {
    let mut state = State::default();
    let mut out = Vec::new();
    for line in source.lines() {
        state.line(line, width, &UNICODE, &mut out);
    }
    out
}

fn texts(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|row| row.text.as_str()).collect()
}

#[test]
fn bullets_hang_their_continuation_rows() {
    let rows = render("- first item wraps onto a second row", 16);
    assert_eq!(
        texts(&rows),
        ["• first item", "  wraps onto a", "  second row"]
    );
    assert_eq!(rows[0].spans, [(0..3, Tone::Muted)]);
}

#[test]
fn nested_and_numbered_items_indent() {
    let rows = render("  - nested\n12. twelfth", 30);
    assert_eq!(texts(&rows), ["  • nested", "12. twelfth"]);
}

#[test]
fn headings_quotes_and_rules() {
    let rows = render("## Plan\n> careful here\n---\n#hashtag", 12);
    assert_eq!(texts(&rows)[..3], ["Plan", "│ careful", "│ here"]);
    assert_eq!(rows[0].tone, Tone::Heading);
    assert_eq!(rows[1].tone, Tone::Muted);
    assert_eq!(rows[3].text, "─".repeat(12));
    assert_eq!(rows[4].text, "#hashtag");
}

#[test]
fn inline_spans_survive_wrapping() {
    let rows = render("run `cargo test --locked` now", 13);
    assert_eq!(texts(&rows), ["run cargo", "test --locked", "now"]);
    assert_eq!(rows[0].spans, [(4..9, Tone::Accent)]);
    assert_eq!(rows[1].spans, [(0..13, Tone::Accent)]);
    assert!(rows[2].spans.is_empty());
}

#[test]
fn links_keep_their_url_muted() {
    let rows = render("see [docs](https://x.dev)", 60);
    assert_eq!(rows[0].text, "see docs (https://x.dev)");
    assert_eq!(rows[0].spans, [(4..8, Tone::Accent), (8..24, Tone::Muted)]);
}

#[test]
fn fences_draw_a_padded_panel_with_highlighting() {
    let rows = render("```rust\nlet n = \"é\\\"\"; // x\n```", 30);
    assert!(rows.iter().all(|row| row.tone == Tone::Code));
    assert!(rows.iter().all(|row| text::width(&row.text) == 30));
    assert_eq!(rows[0].spans, [(1..5, Tone::Muted)]);
    let code = &rows[1];
    let tones: Vec<Tone> = code.spans.iter().map(|(_, tone)| *tone).collect();
    assert_eq!(tones, [Tone::Keyword, Tone::String, Tone::Muted]);
}

#[test]
fn unmatched_markers_stay_literal() {
    assert_eq!(render("a ** b ` c [d]", 40)[0].text, "a ** b ` c [d]");
}
