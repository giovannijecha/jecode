use super::{
    line::Line,
    theme::{CODE_NUMBER, CODE_STRING, CODE_SURFACE, CODE_TEXT, KEYWORD, MUTED, STRONG, Style},
};

mod literals;
mod markdown;
mod markup;
mod perl;
mod profiles;

use literals::{char_literal, list_lead, number, raw_end, raw_prefix, yaml_key};
use profiles::{PROFILES, Profile};

#[derive(Clone, Copy)]
enum Syntax {
    Code(&'static Profile),
    Markup,
    Markdown,
}

#[derive(Clone, Copy)]
enum Mode {
    Plain,
    Quote { close: &'static str, escapes: bool },
    Comment(usize),
    Raw(usize),
    Delimited(perl::Delimited),
}

// A display lexer, not a parser: one engine reads a language profile and keeps
// quote and comment state across lines. Unsupported languages remain ordinary text.
#[derive(Clone)]
pub struct Highlighter {
    syntax: Option<Syntax>,
    mode: Mode,
    markup: markup::Markup,
}

impl Highlighter {
    /// A fenced-code label or a file extension, in any case.
    pub fn new(label: &str) -> Self {
        let name = label
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let name = name.as_str();
        let listed = |names: &str| names.split(' ').any(|listed| listed == name);
        let syntax = if listed(profiles::MARKUP) {
            Some(Syntax::Markup)
        } else if listed(profiles::MARKDOWN) {
            Some(Syntax::Markdown)
        } else {
            PROFILES
                .iter()
                .find(|profile| listed(profile.names))
                .map(Syntax::Code)
        };
        Self {
            syntax,
            mode: Mode::Plain,
            markup: markup::Markup::default(),
        }
    }

    pub fn for_path(path: &str) -> Self {
        Self::new(
            std::path::Path::new(path)
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or(""),
        )
    }

    /// Markup wraps before its tags and compares whole tags.
    pub fn markup(&self) -> bool {
        matches!(self.syntax, Some(Syntax::Markup))
    }

    pub fn line(&mut self, value: &str) -> Line {
        let profile = match self.syntax {
            None => return Line::new(value, CODE_TEXT).on(CODE_SURFACE),
            Some(Syntax::Markup) => return self.markup.line(value),
            Some(Syntax::Markdown) => return markdown::line(value),
            Some(Syntax::Code(profile)) => profile,
        };
        let mut line = Line::default().on(CODE_SURFACE);
        let mut position = 0;
        while position < value.len() {
            let tail = &value[position..];
            let (length, style) = match self.mode {
                Mode::Quote { close, escapes } => {
                    let (length, closed) =
                        quote_end(tail, close, escapes.then_some(profile.escape));
                    if closed {
                        self.mode = Mode::Plain;
                    }
                    (length, CODE_STRING)
                }
                Mode::Comment(depth) => {
                    let (length, depth) = comment_end(tail, profile, depth);
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
                Mode::Delimited(state) => {
                    let (length, state) = perl::body(tail, state);
                    self.mode = state.map_or(Mode::Plain, Mode::Delimited);
                    (length, CODE_STRING)
                }
                Mode::Plain => self.token(profile, value, position),
            };
            line.push(&tail[..length], style);
            position += length;
        }
        line
    }

    fn token(&mut self, profile: &Profile, value: &str, position: usize) -> (usize, Style) {
        let tail = &value[position..];
        let before = &value[..position];
        let first = tail.chars().next().unwrap();
        let previous = before.chars().next_back();
        let after_word = previous.is_some_and(|character| word(character) || character == '$');
        let at_start = before.trim_start().is_empty();
        if let Some((open, _, _)) = profile.block
            && tail.starts_with(open)
        {
            let (length, depth) = comment_end(&tail[open.len()..], profile, 1);
            self.mode = if depth == 0 {
                Mode::Plain
            } else {
                Mode::Comment(depth)
            };
            return (open.len() + length, MUTED);
        }
        if profile.perl {
            if first == '#' && previous == Some('$') {
                return (1, CODE_TEXT);
            }
            if let Some((length, state)) = perl::literal(tail, before) {
                self.mode = state.map_or(Mode::Plain, Mode::Delimited);
                return (length, CODE_STRING);
            }
        }
        if matches!(before.trim_start(), "" | "@")
            && profile.line_start_comments.iter().any(|marker| {
                tail.get(..marker.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(marker))
                    // `REM` ends at a word boundary; `::` needs none.
                    && !(marker.starts_with(char::is_alphabetic)
                        && tail[marker.len()..].starts_with(char::is_alphanumeric))
            })
        {
            return (tail.len(), MUTED);
        }
        if profile
            .line_comments
            .iter()
            .any(|marker| tail.starts_with(marker))
            && (!profile.spaced || previous.is_none_or(char::is_whitespace))
        {
            return (tail.len(), MUTED);
        }
        if profile.preprocessor && at_start && first == '#' {
            let directive = tail[1..].trim_start();
            let letters = directive.bytes().take_while(u8::is_ascii_lowercase).count();
            if letters > 0 {
                return (tail.len() - directive.len() + letters, KEYWORD);
            }
        }
        if profile.rust {
            if let Some((prefix, hashes)) = raw_prefix(tail).filter(|_| !after_word) {
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
        if let Some((open, close)) = profile.long_strings
            && tail.starts_with(open)
        {
            let (length, closed) = quote_end(&tail[open.len()..], close, None);
            if !closed {
                self.mode = Mode::Quote {
                    close,
                    escapes: false,
                };
            }
            return (open.len() + length, CODE_STRING);
        }
        let prefix = if profile.string_prefixes && !after_word {
            tail.bytes()
                .take(2)
                .take_while(|byte| b"rRbBuUfF".contains(byte))
                .count()
        } else {
            0
        };
        let value = before.trim_end();
        let opens = !profile.value_quotes
            || value.is_empty()
            || value.ends_with([':', '=', '-', '[', '{', ',']);
        if let Some(quote) = tail[prefix..]
            .chars()
            .next()
            .filter(|&quote| opens && profile.quotes.contains(quote))
        {
            let close = closer(
                quote,
                profile.triple && tail[prefix..].starts_with(closer(quote, true)),
            );
            let escapes = Some(quote) != profile.verbatim && !tail[..prefix].contains(['r', 'R']);
            let open = prefix + close.len();
            let (length, closed) =
                quote_end(&tail[open..], close, escapes.then_some(profile.escape));
            if !closed {
                self.mode = Mode::Quote { close, escapes };
            }
            return (open + length, CODE_STRING);
        }
        if profile.hex_colors && first == '#' {
            let digits = tail[1..].bytes().take_while(u8::is_ascii_hexdigit).count();
            if (3..=8).contains(&digits) && !tail[1 + digits..].starts_with(ascii_word) {
                return (1 + digits, CODE_NUMBER);
            }
        }
        if profile.at_rules && first == '@' {
            let name = tail[1..]
                .bytes()
                .take_while(|byte| byte.is_ascii_alphabetic() || *byte == b'-')
                .count();
            if name > 0 {
                return (1 + name, KEYWORD);
            }
        }
        if profile.yaml_keys
            && list_lead(before)
            && let Some(length) = yaml_key(tail)
        {
            return (length, KEYWORD);
        }
        if profile.ini_keys && at_start {
            if first == '['
                && let Some(end) = tail.find(']')
            {
                return (end + 1, STRONG.bold());
            }
            let key = tail
                .bytes()
                .take_while(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(byte))
                .count();
            if key > 0 && tail[key..].trim_start().starts_with('=') {
                return (key, KEYWORD);
            }
        }
        if first.is_ascii_digit() && !after_word {
            return (number(tail), CODE_NUMBER);
        }
        let dollar = profile.dollar && first == '$' && tail[1..].starts_with(word);
        if first.is_alphabetic() || first == '_' || dollar {
            let length = tail
                .char_indices()
                .skip(1)
                .find(|&(_, character)| !(word(character) || profile.dollar && character == '$'))
                .map_or(tail.len(), |(index, _)| index);
            let name = &tail[..length];
            let keyword = profile.keywords.split(' ').any(|keyword| {
                keyword == name || profile.caseless && keyword.eq_ignore_ascii_case(name)
            });
            return (length, if keyword { KEYWORD } else { CODE_TEXT });
        }
        (first.len_utf8(), CODE_TEXT)
    }
}

fn word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn ascii_word(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

/// The closing marker of a quote, as a static string the lexer can keep.
fn closer(quote: char, triple: bool) -> &'static str {
    match (quote, triple) {
        ('"', false) => "\"",
        ('"', true) => "\"\"\"",
        ('\'', false) => "'",
        ('\'', true) => "'''",
        (_, false) => "`",
        (_, true) => "```",
    }
}

fn quote_end(value: &str, close: &str, escape: Option<char>) -> (usize, bool) {
    let mut characters = value.char_indices();
    while let Some((index, character)) = characters.next() {
        if Some(character) == escape {
            characters.next();
        } else if value[index..].starts_with(close) {
            return (index + close.len(), true);
        }
    }
    (value.len(), false)
}

fn comment_end(value: &str, profile: &Profile, mut depth: usize) -> (usize, usize) {
    let Some((open, close, nested)) = profile.block else {
        return (value.len(), 0);
    };
    let mut position = 0;
    while position < value.len() {
        let tail = &value[position..];
        if nested && tail.starts_with(open) {
            depth += 1;
            position += open.len();
        } else if tail.starts_with(close) {
            depth -= 1;
            position += close.len();
            if depth == 0 {
                return (position, 0);
            }
        } else {
            position += tail.chars().next().unwrap().len_utf8();
        }
    }
    (position, depth)
}

#[cfg(test)]
mod tests;
