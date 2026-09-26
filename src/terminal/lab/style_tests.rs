use super::*;

#[test]
fn truecolor_matches_jecode_identity() {
    assert_eq!(
        Tone::Accent.sgr(ColorDepth::TrueColor),
        "\x1b[0;38;2;122;162;247m"
    );
    assert_eq!(
        Tone::Heading.sgr(ColorDepth::TrueColor),
        "\x1b[0;1;38;2;220;228;240m"
    );
    assert_eq!(
        Tone::User.sgr(ColorDepth::TrueColor),
        "\x1b[0;38;2;226;232;240;48;2;40;46;57m"
    );
    assert_eq!(
        Tone::Keyword.sgr(ColorDepth::TrueColor),
        "\x1b[0;38;2;187;154;247m"
    );
}

#[test]
fn fallbacks_use_their_own_code_space() {
    assert_eq!(Tone::Accent.sgr(ColorDepth::Ansi256), "\x1b[0;38;5;111m");
    assert_eq!(Tone::Added.sgr(ColorDepth::Ansi16), "\x1b[0;32m");
    assert_eq!(Tone::User.sgr(ColorDepth::Ansi16), "\x1b[0m");
    assert_eq!(Tone::Cursor.sgr(ColorDepth::Ansi16), "\x1b[0;7m");
}

#[test]
fn no_color_keeps_only_attributes() {
    assert_eq!(Tone::Accent.sgr(ColorDepth::None), "");
    assert_eq!(Tone::Heading.sgr(ColorDepth::None), "\x1b[0;1m");
    assert_eq!(Tone::Cursor.sgr(ColorDepth::None), "\x1b[0;7m");
    let mut row = Row::new("a ", Tone::Text);
    row.push("b", Tone::Heading).push(" c", Tone::Muted);
    assert_eq!(row.paint(ColorDepth::None), "a \x1b[0;1mb\x1b[0m c\x1b[0m");
    assert_eq!(
        Row::new("plain", Tone::Muted).paint(ColorDepth::None),
        "plain"
    );
}

#[test]
fn overlay_spans_restore_the_base() {
    let mut row = Row::new("x ", Tone::Code);
    row.push("fn", Tone::Keyword).push(" y", Tone::Code);
    let base = Tone::Code.sgr(ColorDepth::Ansi256);
    let keyword = "\x1b[38;5;141m";
    assert_eq!(
        row.paint(ColorDepth::Ansi256),
        format!("{base}x {keyword}fn{base} y\x1b[0m")
    );
    assert_eq!(row.spans, [(2..4, Tone::Keyword)]);
}

#[test]
fn accent_inside_a_panel_keeps_the_panel_background() {
    let mut row = Row::new(" ", Tone::User);
    row.push("›", Tone::Accent).push(" hi", Tone::User);
    let painted = row.paint(ColorDepth::TrueColor);
    assert!(painted.contains("\x1b[38;2;122;162;247m›"), "{painted:?}");
}

#[test]
fn indent_leaves_the_margin_uncolored() {
    let row = indent(Row::new("code", Tone::Code), " ");
    assert_eq!(row.text, " code");
    assert_eq!(row.spans, [(0..1, Tone::Text)]);
    let mut text = Row::new("a", Tone::Text);
    text.push("b", Tone::Accent);
    assert_eq!(indent(text, "  ").spans, [(3..4, Tone::Accent)]);
}
