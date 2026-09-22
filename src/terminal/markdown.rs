//! Small owned presentation subset, not a general Markdown or syntax parser.
use super::{
    style::{Row, Tone, lines, pad},
    text,
};
#[cfg(test)]
pub fn render(input: &str, width: usize) -> Vec<Row> {
    let clean = text::safe(input);
    let mut output = Vec::new();
    let mut state = State::default();
    for line in clean.lines() {
        state.line(line, width, &mut output);
    }
    output
}

/// The supported grammar only carries fenced-code state across source lines.
#[derive(Clone, Copy, Default)]
pub struct State {
    code: bool,
}
impl State {
    /// Append one sanitized source line. A preview uses a copy of this state.
    pub fn line(&mut self, line: &str, width: usize, output: &mut Vec<Row>) {
        if let Some(language) = line.trim_start().strip_prefix("```") {
            if !self.code {
                output.extend(lines(
                    if language.is_empty() {
                        "code"
                    } else {
                        language
                    },
                    width,
                    Tone::Muted,
                ));
            }
            self.code = !self.code;
            return;
        }
        if self.code {
            for line in text::wrap(line, width.saturating_sub(2).max(1)) {
                let mut row = Row::new(format!(" {line}"), Tone::Code);
                highlight(&mut row);
                output.push(pad(row, width));
            }
        } else {
            let (body, tone) = if let Some(body) = line
                .strip_prefix("### ")
                .or_else(|| line.strip_prefix("## "))
                .or_else(|| line.strip_prefix("# "))
            {
                (body, Tone::Heading)
            } else {
                (line, Tone::Text)
            };
            let (plain, mut spans) = inline(body);
            let wrapped = text::wrap(&plain, width.max(1));
            if wrapped.concat() != plain {
                // An oversized indivisible Unicode run was replaced for display;
                // original byte ranges must not style unrelated following text.
                spans.clear();
            }
            let mut start = 0;
            for line in wrapped {
                let end = start + line.len();
                let mut row = Row::new(&line, tone);
                for (range, tone) in &spans {
                    let a = range.start.max(start);
                    let b = range.end.min(end);
                    if a < b && line.is_char_boundary(a - start) && line.is_char_boundary(b - start)
                    {
                        row.spans.push((a - start..b - start, *tone));
                    }
                }
                output.push(row);
                start = end;
            }
        }
    }
}
fn inline(input: &str) -> (String, Vec<(std::ops::Range<usize>, Tone)>) {
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut rest = input;
    while !rest.is_empty() {
        let marker = if rest.starts_with("**") {
            "**"
        } else if rest.starts_with('`') {
            "`"
        } else {
            ""
        };
        if !marker.is_empty()
            && let Some(end) = rest[marker.len()..].find(marker)
        {
            let start = text.len();
            text.push_str(&rest[marker.len()..marker.len() + end]);
            spans.push((
                start..text.len(),
                if marker == "`" {
                    Tone::Accent
                } else {
                    Tone::Heading
                },
            ));
            rest = &rest[end + marker.len() * 2..];
        } else {
            let ch = rest.chars().next().unwrap();
            text.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    (text, spans)
}
fn highlight(row: &mut Row) {
    let bytes = row.text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        let tone = if bytes[index..].starts_with(b"//") {
            row.spans.push((index..bytes.len(), Tone::Muted));
            break;
        } else if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index += 1;
                    if index < bytes.len() {
                        index += row.text[index..].chars().next().unwrap().len_utf8();
                    }
                } else if bytes[index] == b'"' {
                    index += 1;
                    break;
                } else {
                    index += row.text[index..].chars().next().unwrap().len_utf8();
                }
            }
            Some(Tone::String)
        } else if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            matches!(
                &row.text[start..index],
                "fn" | "let"
                    | "mut"
                    | "pub"
                    | "use"
                    | "return"
                    | "if"
                    | "else"
                    | "for"
                    | "in"
                    | "const"
                    | "function"
                    | "async"
                    | "await"
                    | "true"
                    | "false"
            )
            .then_some(Tone::Keyword)
        } else if bytes[index].is_ascii_digit() {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            Some(Tone::Number)
        } else {
            index += row.text[index..].chars().next().unwrap().len_utf8();
            None
        };
        if let Some(tone) = tone {
            row.spans.push((start..index, tone));
        }
    }
}
