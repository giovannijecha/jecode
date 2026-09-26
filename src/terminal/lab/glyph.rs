//! The only place UI symbols are spelled. Views ask for a role, never a
//! literal, so the ASCII fallback is total. Layout glyphs have the same width in
//! both sets, so layout never depends on which set is active.

pub struct Glyphs {
    /// Horizontal rule above and below the composer.
    pub rule: &'static str,
    pub prompt: &'static str,
    pub ok: &'static str,
    pub fail: &'static str,
    pub warn: &'static str,
    /// Separator between facts, used with a space on each side.
    pub dot: &'static str,
    pub ellipsis: &'static str,
    /// Tree: a step with siblings below, the last step, and the continuation.
    pub branch: &'static str,
    pub last: &'static str,
    pub stem: &'static str,
    /// List item marker.
    pub bullet: &'static str,
    /// Key hint for moving a selection; free width, used only in hints.
    pub arrows: &'static str,
}

pub const UNICODE: Glyphs = Glyphs {
    rule: "─",
    prompt: "›",
    ok: "✓",
    fail: "✗",
    warn: "!",
    dot: "·",
    ellipsis: "…",
    branch: "├─",
    last: "└─",
    stem: "│",
    bullet: "•",
    arrows: "↑↓",
};

pub const ASCII: Glyphs = Glyphs {
    rule: "-",
    prompt: ">",
    ok: "+",
    fail: "x",
    warn: "!",
    dot: "-",
    ellipsis: "...",
    branch: "|-",
    last: "`-",
    stem: "|",
    bullet: "-",
    arrows: "Up/Down",
};

pub fn glyphs(ascii: bool) -> &'static Glyphs {
    if ascii { &ASCII } else { &UNICODE }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::lab::text::width;

    #[test]
    fn sets_share_cell_widths_except_free_hints() {
        let cells = |g: &Glyphs| {
            [
                g.rule, g.prompt, g.ok, g.fail, g.warn, g.dot, g.branch, g.last, g.stem, g.bullet,
            ]
            .map(width)
        };
        assert_eq!(cells(&UNICODE), cells(&ASCII));
        assert!(ASCII.ellipsis.is_ascii() && ASCII.arrows.is_ascii());
    }
}
