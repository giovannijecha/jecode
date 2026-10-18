use super::*;
use crate::tui::theme::{BAD, MUTED, WARNING};

#[test]
fn source_feedback_is_italic_at_the_left_edge_and_wraps_without_a_glyph() {
    for (level, style) in [
        (Level::Info, MUTED.italic()),
        (Level::Warning, WARNING.italic()),
        (Level::Error, BAD.italic()),
    ] {
        let lines = lines(level, "A brief result", 80);
        assert_eq!(lines[0].plain(), "A brief result");
        assert_eq!(lines[0].spans[0].style, style);
        assert_eq!(lines[0].background, None);
    }
    let wrapped = lines(Level::Error, "The session could not be saved", 15);
    assert!(wrapped.len() > 1);
    assert!(wrapped.iter().all(|line| text::cells(&line.plain()) <= 15));
}

#[test]
fn levels_follow_message_kinds() {
    assert_eq!(Level::from(Kind::Notice), Level::Info);
    assert_eq!(Level::from(Kind::Warning), Level::Warning);
    assert_eq!(Level::from(Kind::Error), Level::Error);
}
