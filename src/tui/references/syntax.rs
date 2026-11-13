//! The text of a reference, which is all a message carries. `$name` names a personal
//! skill. `@path` names a workspace file by its relative path with `/` separators;
//! a path holding a space, a quote or another character that would end or blur the
//! token is written `@"path"`, with `\"` and `\\` escaped inside the quotes.
//!
//! Nothing here converts text: the parser only finds the reference under the caret, so
//! `$` and `@` typed elsewhere stay literal.

use crate::attachments::MARKER;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sigil {
    Skill,
    File,
}

impl Sigil {
    pub fn of(text: &str) -> Option<Self> {
        match text {
            "$" => Some(Self::Skill),
            "@" => Some(Self::File),
            _ => None,
        }
    }

    pub fn character(self) -> char {
        match self {
            Self::Skill => '$',
            Self::File => '@',
        }
    }
}

/// One reference in a draft: its byte range, sigil included, and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub sigil: Sigil,
    pub start: usize,
    pub end: usize,
    pub value: String,
}

/// Where a reference may start or end: the edge of the text, whitespace or an
/// attachment.
pub fn boundary(character: Option<char>) -> bool {
    character.is_none_or(|character| character.is_whitespace() || character == MARKER)
}

/// Whether a sigil typed at `cursor` opens a selector: it stands alone, at a word
/// boundary on both sides. `a@b`, `\$`, `$HOME` typed before a word and `($x)` stay text.
pub fn opens(text: &str, cursor: usize) -> bool {
    boundary(text[..cursor].chars().next_back()) && boundary(text[cursor..].chars().next())
}

/// Prose punctuation after a reference that is not part of it.
fn trailing(character: char) -> bool {
    matches!(
        character,
        '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '\'' | '"'
    )
}

fn skill_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
}

/// The reference starting at byte `start`, which holds a sigil at a boundary.
fn token(text: &str, start: usize, sigil: Sigil) -> Option<Token> {
    let rest = &text[start + 1..];
    match sigil {
        Sigil::Skill => {
            let length = rest
                .find(|character| !skill_character(character))
                .unwrap_or(rest.len());
            // Sentence punctuation may follow, but only before a boundary: `$5.00` is text.
            let tail = &rest[length..];
            let punctuation = tail
                .find(|character| !trailing(character))
                .unwrap_or(tail.len());
            boundary(tail[punctuation..].chars().next()).then(|| Token {
                sigil,
                start,
                end: start + 1 + length,
                value: rest[..length].into(),
            })
        }
        Sigil::File if rest.starts_with('"') => {
            let mut value = String::new();
            let mut escaped = false;
            for (offset, character) in rest.char_indices().skip(1) {
                match character {
                    '\n' | MARKER => return None,
                    _ if escaped => {
                        value.push(character);
                        escaped = false;
                    }
                    '\\' => escaped = true,
                    '"' => {
                        return Some(Token {
                            sigil,
                            start,
                            end: start + 2 + offset,
                            value,
                        });
                    }
                    _ => value.push(character),
                }
            }
            None
        }
        Sigil::File => {
            let mut length = rest
                .find(|character: char| boundary(Some(character)))
                .unwrap_or(rest.len());
            while let Some(last) = rest[..length].chars().next_back().filter(|c| trailing(*c)) {
                length -= last.len_utf8();
            }
            Some(Token {
                sigil,
                start,
                end: start + 1 + length,
                value: rest[..length].into(),
            })
        }
    }
}

/// Every reference in `text`, in order.
pub fn tokens(text: &str) -> Vec<Token> {
    let mut found = Vec::new();
    let mut from = 0;
    for (index, character) in text.char_indices() {
        if index < from {
            continue;
        }
        let sigil = match character {
            '$' => Sigil::Skill,
            '@' => Sigil::File,
            _ => continue,
        };
        if !boundary(text[..index].chars().next_back()) {
            continue;
        }
        if let Some(token) = token(text, index, sigil) {
            from = token.end;
            found.push(token);
        }
    }
    found
}

/// The references the chat composer draws as chips: every nonempty reference, unless
/// the draft is a slash command, whose arguments stay plain text.
pub fn chips(text: &str) -> Vec<std::ops::Range<usize>> {
    if text.starts_with('/') {
        return Vec::new();
    }
    tokens(text)
        .into_iter()
        .filter(|token| !token.value.is_empty())
        .map(|token| token.start..token.end)
        .collect()
}

/// The reference the caret is on, from just before its sigil to just after its end.
pub fn at(text: &str, cursor: usize) -> Option<Token> {
    tokens(text)
        .into_iter()
        .find(|token| token.start <= cursor && cursor <= token.end)
}

pub fn skill(name: &str) -> String {
    format!("${name}")
}

/// Characters a bare `@path` may hold: anything that cannot end the token or read as
/// the prose around it.
fn bare(character: char) -> bool {
    !character.is_whitespace()
        && character != MARKER
        && !"\"'`()[]{}<>,;:|&$@!?*#\\".contains(character)
}

/// `@path`, or `@"path"` when it needs quotes. Control characters and the
/// attachment marker cannot be represented in the editor's text.
pub fn file(path: &str) -> Option<String> {
    if path.is_empty() || path.chars().any(|ch| ch.is_control() || ch == MARKER) {
        return None;
    }
    if path.chars().all(bare) && !path.ends_with('.') {
        return Some(format!("@{path}"));
    }
    let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
    Some(format!("@\"{escaped}\""))
}

/// A typed or pasted path as the file filter reads it: separators as `/`, without a
/// leading `@`, `./` or surrounding quotes.
pub fn typed_path(filter: &str) -> String {
    let mut value = filter.trim();
    value = value.strip_prefix('@').unwrap_or(value);
    let quoted = value.len() >= 2 && value.starts_with('"') && value.ends_with('"');
    if quoted {
        value = &value[1..value.len() - 1];
    }
    let mut normalized = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' {
            match characters.peek().copied() {
                Some('"' | '\\') if quoted => normalized.push(characters.next().unwrap()),
                _ => normalized.push('/'),
            }
        } else {
            normalized.push(character);
        }
    }
    let mut value = normalized;
    while let Some(rest) = value.strip_prefix("./") {
        value = rest.into();
    }
    value
}
