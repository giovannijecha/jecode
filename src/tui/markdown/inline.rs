use crate::tui::{
    line::Line,
    theme::{
        EMPHASIS, INLINE_CODE, ITALIC, ITALIC_STRIKE, ITALIC_STRONG, ITALIC_STRONG_STRIKE, LINK,
        LINK_DESTINATION, LINK_ITALIC, LINK_ITALIC_STRIKE, LINK_ITALIC_STRONG,
        LINK_ITALIC_STRONG_STRIKE, LINK_STRIKE, LINK_STRONG, LINK_STRONG_STRIKE, STRIKE,
        STRONG_STRIKE,
    },
};

const STRONG: u8 = 1;
const SLANT: u8 = 2;
const DELETED: u8 = 4;
const LINKED: u8 = 8;
const MAX_DEPTH: usize = 8;

pub fn render(value: &str, base: &'static str) -> Line {
    let mut parser = Parser {
        line: Line::default(),
        // Failed opening markers must not repeatedly scan an arbitrarily long suffix.
        search_left: value.len().saturating_mul(16),
    };
    parser.segment(value, base, u8::from(base == EMPHASIS) * STRONG, 0);
    parser.line
}

struct Parser {
    line: Line,
    search_left: usize,
}

impl Parser {
    fn segment(&mut self, value: &str, base: &'static str, flags: u8, depth: usize) {
        let mut position = 0;
        while position < value.len() {
            let Some(offset) = value[position..].find(['\\', '\u{0060}', '*', '_', '~', '['])
            else {
                self.line.push(&value[position..], style(base, flags));
                return;
            };
            let start = position + offset;
            self.line.push(&value[position..start], style(base, flags));
            let tail = &value[start..];

            if let Some(rest) = tail.strip_prefix('\\') {
                if let Some(next) = rest.chars().next()
                    && next.is_ascii_punctuation()
                {
                    self.line.push(&rest[..next.len_utf8()], style(base, flags));
                    position = start + 1 + next.len_utf8();
                } else {
                    self.line.push("\\", style(base, flags));
                    position = start + 1;
                }
                continue;
            }

            if tail.starts_with('\u{0060}') {
                let count = run(tail, b'\x60');
                if let Some(end) = self.code_end(&tail[count..], count) {
                    self.line.push(&tail[count..count + end], INLINE_CODE);
                    position = start + count + end + count;
                } else {
                    self.line.push(&tail[..count], style(base, flags));
                    position = start + count;
                }
                continue;
            }

            if tail.starts_with('[')
                && depth < MAX_DEPTH
                && !value[..start].ends_with('!')
                && let Some((label_end, destination_end)) = self.link_end(tail)
            {
                let label = &tail[1..label_end];
                let destination = &tail[label_end + 2..destination_end];
                self.segment(label, base, flags | LINKED, depth + 1);
                self.line.push(" (", style(base, flags));
                self.line.push(destination, LINK_DESTINATION);
                self.line.push(")", style(base, flags));
                position = start + destination_end + 1;
                continue;
            }

            let marker = tail.as_bytes()[0];
            if matches!(marker, b'*' | b'_' | b'~') {
                let count = run(tail, marker);
                let width = if marker == b'~' {
                    (count == 2).then_some(2)
                } else {
                    (count <= 3).then_some(count)
                };
                if let Some(width) = width
                    && depth < MAX_DEPTH
                    && opening_allowed(value, start, marker, width)
                    && let Some(end) = self.delimiter_end(&tail[width..], marker, width)
                {
                    let content = &tail[width..width + end];
                    let added = match (marker, width) {
                        (b'~', _) => DELETED,
                        (_, 1) => SLANT,
                        (_, 2) => STRONG,
                        _ => STRONG | SLANT,
                    };
                    self.segment(content, base, flags | added, depth + 1);
                    position = start + width + end + width;
                    continue;
                }
                self.line.push(&tail[..count], style(base, flags));
                position = start + count;
                continue;
            }

            self.line.push("[", style(base, flags));
            position = start + 1;
        }
    }

    fn spend(&mut self, amount: usize) -> bool {
        if amount > self.search_left {
            self.search_left = 0;
            false
        } else {
            self.search_left -= amount;
            true
        }
    }

    fn code_end(&mut self, value: &str, width: usize) -> Option<usize> {
        let mut position = 0;
        while let Some(offset) = value[position..].find('\u{0060}') {
            let start = position + offset;
            let count = run(&value[start..], b'\x60');
            if !self.spend(start + count - position) {
                return None;
            }
            if count == width {
                return Some(start);
            }
            position = start + count;
        }
        self.spend(value.len() - position).then_some(())?;
        None
    }

    fn delimiter_end(&mut self, value: &str, marker: u8, width: usize) -> Option<usize> {
        let mut position = 0;
        while position < value.len() {
            let Some(offset) = value[position..].find(['\\', '\u{0060}', '*', '_', '~']) else {
                self.spend(value.len() - position);
                return None;
            };
            let start = position + offset;
            let byte = value.as_bytes()[start];
            if !self.spend(start + 1 - position) {
                return None;
            }
            if byte == b'\\' {
                position = start + 1;
                if let Some(character) = value[position..].chars().next() {
                    position += character.len_utf8();
                }
            } else if byte == b'\x60' {
                let count = run(&value[start..], b'\x60');
                position = start + count;
                if let Some(end) = self.code_end(&value[position..], count) {
                    position += end + count;
                }
            } else {
                let count = run(&value[start..], byte);
                if byte == marker && count == width && closing_allowed(value, start, marker, width)
                {
                    return Some(start);
                }
                position = start + count;
            }
        }
        None
    }

    fn link_end(&mut self, value: &str) -> Option<(usize, usize)> {
        let mut position = 1;
        let mut brackets = 1usize;
        while position < value.len() {
            let character = value[position..].chars().next()?;
            if !self.spend(character.len_utf8()) {
                return None;
            }
            if character == '\\' {
                position += 1;
                position += value[position..].chars().next().map_or(0, char::len_utf8);
                continue;
            }
            if character == '\u{0060}' {
                let count = run(&value[position..], b'\x60');
                position += count;
                if let Some(end) = self.code_end(&value[position..], count) {
                    position += end + count;
                }
                continue;
            }
            if character == '[' {
                brackets += 1;
            } else if character == ']' {
                brackets -= 1;
                if brackets == 0 {
                    break;
                }
            }
            position += character.len_utf8();
        }
        if brackets != 0 || position == 1 || !value[position..].starts_with("](") {
            return None;
        }
        let label_end = position;
        position += 2;
        let destination_start = position;
        let mut parentheses = 1usize;
        while position < value.len() {
            let character = value[position..].chars().next()?;
            if !self.spend(character.len_utf8()) {
                return None;
            }
            if character == '\\' {
                position += 1;
                position += value[position..].chars().next().map_or(0, char::len_utf8);
                continue;
            }
            if character == '(' {
                parentheses += 1;
                if parentheses > MAX_DEPTH {
                    return None;
                }
            } else if character == ')' {
                parentheses -= 1;
                if parentheses == 0 {
                    let destination = &value[destination_start..position];
                    return (!destination.is_empty()
                        && !destination.chars().any(char::is_whitespace))
                    .then_some((label_end, position));
                }
            }
            position += character.len_utf8();
        }
        None
    }
}

fn run(value: &str, marker: u8) -> usize {
    value.bytes().take_while(|&byte| byte == marker).count()
}

fn opening_allowed(value: &str, start: usize, marker: u8, width: usize) -> bool {
    let after = value[start + width..].chars().next();
    if after.is_none_or(char::is_whitespace) {
        return false;
    }
    marker != b'_' || !inside_word(value[..start].chars().next_back(), after)
}

fn closing_allowed(value: &str, start: usize, marker: u8, width: usize) -> bool {
    let before = value[..start].chars().next_back();
    if before.is_none_or(char::is_whitespace) {
        return false;
    }
    marker != b'_' || !inside_word(before, value[start + width..].chars().next())
}

fn inside_word(before: Option<char>, after: Option<char>) -> bool {
    before.is_some_and(char::is_alphanumeric) && after.is_some_and(char::is_alphanumeric)
}

fn style(base: &'static str, flags: u8) -> &'static str {
    match flags {
        0 => base,
        STRONG => EMPHASIS,
        SLANT => ITALIC,
        3 => ITALIC_STRONG,
        DELETED => STRIKE,
        5 => STRONG_STRIKE,
        6 => ITALIC_STRIKE,
        7 => ITALIC_STRONG_STRIKE,
        LINKED => LINK,
        9 => LINK_STRONG,
        10 => LINK_ITALIC,
        11 => LINK_ITALIC_STRONG,
        12 => LINK_STRIKE,
        13 => LINK_STRONG_STRIKE,
        14 => LINK_ITALIC_STRIKE,
        _ => LINK_ITALIC_STRONG_STRIKE,
    }
}

#[cfg(test)]
mod tests;
