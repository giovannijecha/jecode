use super::{CODE_STRING, CODE_SURFACE, CODE_TEXT, Line, STRONG};

/// Headings in bold, `inline code` as a string, the rest plain prose.
pub(super) fn line(value: &str) -> Line {
    let mut line = Line::default().on(CODE_SURFACE);
    let marks = value.trim_start_matches(' ');
    let level = marks.bytes().take_while(|&byte| byte == b'#').count();
    if value.len() - marks.len() <= 3
        && (1..=6).contains(&level)
        && marks[level..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
    {
        line.push(value, STRONG.bold());
        return line;
    }
    let mut rest = value;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        if let Some(close) = after.find('`').filter(|&close| close > 0) {
            line.push(&rest[..open], CODE_TEXT);
            line.push(&rest[open..open + close + 2], CODE_STRING);
            rest = &after[close + 1..];
        } else {
            line.push(&rest[..=open], CODE_TEXT);
            rest = after;
        }
    }
    line.push(rest, CODE_TEXT);
    line
}
