use super::{
    line::Line,
    theme::{CODE_BACKGROUND, CODE_NUMBER, CODE_STRING, CODE_TEXT, KEYWORD, MUTED},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    Shell,
    Json,
}

#[derive(Clone, Copy)]
enum Mode {
    Plain,
    Quote(char),
    Comment(usize),
    Raw(usize),
}

// A display lexer, not a parser. Unsupported languages remain ordinary text.
pub struct Highlighter {
    language: Option<Language>,
    mode: Mode,
}

impl Highlighter {
    pub fn new(label: &str) -> Self {
        let language = match label
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "rust" | "rs" => Some(Language::Rust),
            "bash" | "sh" | "shell" => Some(Language::Shell),
            "json" => Some(Language::Json),
            _ => None,
        };
        Self {
            language,
            mode: Mode::Plain,
        }
    }

    pub fn line(&mut self, value: &str) -> Line {
        let Some(language) = self.language else {
            return Line::new(value, CODE_TEXT).on(CODE_BACKGROUND);
        };
        let mut line = Line::default().on(CODE_BACKGROUND);
        let mut position = 0;
        while position < value.len() {
            let tail = &value[position..];
            let (length, style) = match self.mode {
                Mode::Quote(delimiter) => {
                    let (length, closed) = quote_end(tail, delimiter, delimiter != '\'');
                    if closed {
                        self.mode = Mode::Plain;
                    }
                    (length, CODE_STRING)
                }
                Mode::Comment(depth) => {
                    let (length, depth) = comment_end(tail, depth);
                    self.mode = if depth == 0 {
                        Mode::Plain
                    } else {
                        Mode::Comment(depth)
                    };
                    (length, MUTED)
                }
                Mode::Raw(hashes) => {
                    let (length, closed) = raw_end(tail, hashes);
                    if closed {
                        self.mode = Mode::Plain;
                    }
                    (length, CODE_STRING)
                }
                Mode::Plain => self.token(value, position, language),
            };
            line.push(&tail[..length], style);
            position += length;
        }
        line
    }

    fn token(&mut self, value: &str, position: usize, language: Language) -> (usize, &'static str) {
        let tail = &value[position..];
        let first = tail.chars().next().unwrap();
        if language == Language::Rust && tail.starts_with("//")
            || language == Language::Shell
                && first == '#'
                && (position == 0 || value[..position].ends_with(char::is_whitespace))
        {
            return (tail.len(), MUTED);
        }
        if language == Language::Rust && tail.starts_with("/*") {
            let (length, depth) = comment_end(&tail[2..], 1);
            self.mode = if depth == 0 {
                Mode::Plain
            } else {
                Mode::Comment(depth)
            };
            return (length + 2, MUTED);
        }
        if language == Language::Rust {
            if let Some((prefix, hashes)) = raw_prefix(tail) {
                let (length, closed) = raw_end(&tail[prefix..], hashes);
                if !closed {
                    self.mode = Mode::Raw(hashes);
                }
                return (prefix + length, CODE_STRING);
            }
            if let Some(length) = char_literal(tail) {
                return (length, CODE_STRING);
            }
        }
        if first == '"' || language == Language::Shell && first == '\'' {
            let (length, closed) = quote_end(&tail[1..], first, first != '\'');
            if !closed {
                self.mode = Mode::Quote(first);
            }
            return (length + 1, CODE_STRING);
        }
        if first.is_ascii_digit() {
            let length = tail
                .bytes()
                .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.'))
                .count();
            return (length, CODE_NUMBER);
        }
        if first.is_alphabetic() || first == '_' {
            let length = tail
                .char_indices()
                .find(|&(_, character)| !character.is_alphanumeric() && character != '_')
                .map_or(tail.len(), |(index, _)| index);
            let word = &tail[..length];
            let keyword = match language {
                Language::Rust => matches!(
                    word,
                    "as" | "async"
                        | "await"
                        | "break"
                        | "const"
                        | "continue"
                        | "crate"
                        | "dyn"
                        | "else"
                        | "enum"
                        | "extern"
                        | "false"
                        | "fn"
                        | "for"
                        | "if"
                        | "impl"
                        | "in"
                        | "let"
                        | "loop"
                        | "match"
                        | "mod"
                        | "move"
                        | "mut"
                        | "pub"
                        | "ref"
                        | "return"
                        | "self"
                        | "Self"
                        | "static"
                        | "struct"
                        | "super"
                        | "trait"
                        | "true"
                        | "type"
                        | "unsafe"
                        | "use"
                        | "where"
                        | "while"
                ),
                Language::Shell => matches!(
                    word,
                    "case"
                        | "do"
                        | "done"
                        | "elif"
                        | "else"
                        | "esac"
                        | "fi"
                        | "for"
                        | "function"
                        | "if"
                        | "in"
                        | "select"
                        | "then"
                        | "until"
                        | "while"
                ),
                Language::Json => matches!(word, "true" | "false" | "null"),
            };
            return (length, if keyword { KEYWORD } else { CODE_TEXT });
        }
        (first.len_utf8(), CODE_TEXT)
    }
}

fn quote_end(value: &str, delimiter: char, escapes: bool) -> (usize, bool) {
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
        } else if escapes && character == '\\' {
            escaped = true;
        } else if character == delimiter {
            return (index + character.len_utf8(), true);
        }
    }
    (value.len(), false)
}

fn comment_end(value: &str, mut depth: usize) -> (usize, usize) {
    let mut position = 0;
    while position < value.len() {
        let tail = &value[position..];
        if tail.starts_with("/*") {
            depth += 1;
            position += 2;
        } else if tail.starts_with("*/") {
            depth -= 1;
            position += 2;
            if depth == 0 {
                return (position, 0);
            }
        } else {
            position += tail.chars().next().unwrap().len_utf8();
        }
    }
    (position, depth)
}

fn raw_prefix(value: &str) -> Option<(usize, usize)> {
    let start = if value.starts_with("br") {
        2
    } else if value.starts_with('r') {
        1
    } else {
        return None;
    };
    let hashes = value[start..]
        .bytes()
        .take_while(|&byte| byte == b'#')
        .count();
    (value.as_bytes().get(start + hashes) == Some(&b'"')).then_some((start + hashes + 1, hashes))
}

fn raw_end(value: &str, hashes: usize) -> (usize, bool) {
    for (index, _) in value.match_indices('"') {
        let end = index + 1 + hashes;
        if value
            .as_bytes()
            .get(index + 1..end)
            .is_some_and(|tail| tail.iter().all(|&byte| byte == b'#'))
        {
            return (end, true);
        }
    }
    (value.len(), false)
}

fn char_literal(value: &str) -> Option<usize> {
    let tail = value.strip_prefix('\'')?;
    let first = tail.chars().next()?;
    let mut length = 1 + first.len_utf8();
    if first == '\\' {
        let escape = value[length..].chars().next()?;
        length += escape.len_utf8();
        if escape == 'u' && value[length..].starts_with('{') {
            length += value[length..].find('}')? + 1;
        } else if escape == 'x' {
            if !value
                .as_bytes()
                .get(length..length + 2)?
                .iter()
                .all(u8::is_ascii_hexdigit)
            {
                return None;
            }
            length += 2;
        }
    } else if first == '\'' {
        return None;
    }
    value[length..].starts_with('\'').then_some(length + 1)
}

#[cfg(test)]
mod tests;
