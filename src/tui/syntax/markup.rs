use super::{CODE_STRING, CODE_SURFACE, CODE_TEXT, KEYWORD, Line, MUTED, quote_end};
use crate::tui::theme::EMPHASIS;

#[derive(Clone, Default)]
pub(super) struct Markup {
    comment: bool,
    tag: bool,
    quote: Option<char>,
}

impl Markup {
    pub fn line(&mut self, value: &str) -> Line {
        let mut row = Line::default().on(CODE_SURFACE);
        let mut at = 0;
        while at < value.len() {
            let tail = &value[at..];
            let (length, style) = if self.comment || tail.starts_with("<!--") {
                let end = tail.find("-->").map(|end| end + 3);
                self.comment = end.is_none();
                (end.unwrap_or(tail.len()), MUTED)
            } else if let Some(quote) = self.quote {
                let (length, closed) = quote_end(tail, quote, false);
                if closed {
                    self.quote = None;
                }
                (length, CODE_STRING)
            } else if tail.starts_with('<') {
                self.tag = true;
                let prefix = if tail.starts_with("</") || tail.starts_with("<?") {
                    2
                } else {
                    1
                };
                (prefix + identifier(&tail[prefix..]), EMPHASIS)
            } else if self.tag && (tail.starts_with("/>") || tail.starts_with("?>")) {
                self.tag = false;
                (2, CODE_TEXT)
            } else if self.tag && tail.starts_with('>') {
                self.tag = false;
                (1, CODE_TEXT)
            } else if self.tag && (tail.starts_with('"') || tail.starts_with('\'')) {
                let quote = tail.chars().next().unwrap();
                let (length, closed) = quote_end(&tail[1..], quote, false);
                if !closed {
                    self.quote = Some(quote);
                }
                (length + 1, CODE_STRING)
            } else {
                let length = identifier(tail);
                if self.tag && length > 0 && tail[length..].trim_start().starts_with('=') {
                    (length, KEYWORD)
                } else {
                    (tail.chars().next().unwrap().len_utf8(), CODE_TEXT)
                }
            };
            row.push(&tail[..length], style);
            at += length;
        }
        row
    }
}

fn identifier(value: &str) -> usize {
    value
        .char_indices()
        .find(|(_, ch)| !ch.is_alphanumeric() && !matches!(ch, '_' | ':' | '-' | '.'))
        .map_or(value.len(), |(at, _)| at)
}
