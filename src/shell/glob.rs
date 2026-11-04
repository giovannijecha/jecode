//! Pathname expansion for unquoted `*`, `?` and `[...]`. A pattern that
//! matches nothing stays literal, as in POSIX shells without `nullglob`.

use std::path::Path;

/// One pattern character: quoted characters are always literal.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Item {
    Literal(char),
    Meta(char),
}

/// Expands `pattern` relative to `cwd`; `None` when it has no unquoted
/// metacharacter or matches nothing.
pub(super) fn expand(pattern: &[Item], cwd: &Path) -> Option<Vec<String>> {
    if !pattern
        .iter()
        .any(|item| matches!(item, Item::Meta('*' | '?' | '[')))
    {
        return None;
    }
    let components: Vec<&[Item]> = pattern
        .split(|item| matches!(item, Item::Literal('/' | '\\') | Item::Meta('/')))
        .collect();
    let mut prefixes = vec![String::new()];
    for (index, component) in components.iter().enumerate() {
        let last = index + 1 == components.len();
        let literal: Option<String> = component
            .iter()
            .map(|item| match item {
                Item::Literal(ch) | Item::Meta(ch @ ']') => Some(*ch),
                Item::Meta(_) => None,
            })
            .collect();
        let tokens = compile(component);
        let mut next = Vec::new();
        for prefix in &prefixes {
            let joined = |name: &str| {
                if index == 0 {
                    name.to_owned()
                } else {
                    format!("{prefix}/{name}")
                }
            };
            if let Some(name) = &literal {
                next.push(joined(name));
                continue;
            }
            let directory = if index == 0 {
                cwd.to_path_buf()
            } else if prefix.is_empty() || prefix.ends_with(':') {
                // `/x` and `C:/x` start at the root.
                cwd.join(format!("{prefix}/"))
            } else {
                cwd.join(prefix)
            };
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            let mut names: Vec<String> = entries
                .filter_map(Result::ok)
                .filter(|entry| last || entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| {
                    !name.starts_with('.') || component.first() == Some(&Item::Literal('.'))
                })
                .filter(|name| matches(&tokens, &name.chars().collect::<Vec<_>>()))
                .collect();
            names.sort();
            next.extend(names.iter().map(|name| joined(name)));
        }
        prefixes = next;
    }
    // Literal components are not checked until here. A link counts as itself,
    // even when its target is gone.
    let found: Vec<String> = prefixes
        .into_iter()
        .filter(|path| std::fs::symlink_metadata(cwd.join(path)).is_ok())
        .collect();
    (!found.is_empty()).then_some(found)
}

/// Whether a file name matches a pattern such as `*.rs`, for `grep --include`.
pub(super) fn name_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<Item> = pattern
        .chars()
        .map(|ch| match ch {
            '*' | '?' | '[' | ']' => Item::Meta(ch),
            _ => Item::Literal(ch),
        })
        .collect();
    matches(&compile(&pattern), &name.chars().collect::<Vec<_>>())
}

/// One element of a compiled path component.
enum Token {
    Star,
    One,
    Char(char),
    Set(Set),
}

/// A bracket expression.
struct Set {
    negated: bool,
    ranges: Vec<(char, char)>,
}

impl Token {
    fn accepts(&self, ch: char) -> bool {
        match self {
            Self::Star => false,
            Self::One => true,
            Self::Char(expected) => ch == *expected,
            Self::Set(set) => {
                set.ranges
                    .iter()
                    .any(|(low, high)| (*low..=*high).contains(&ch))
                    != set.negated
            }
        }
    }
}

/// A `[` with no closing `]` is an ordinary character.
fn compile(pattern: &[Item]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut at = 0;
    while let Some(item) = pattern.get(at) {
        at += 1;
        tokens.push(match item {
            Item::Meta('*') => Token::Star,
            Item::Meta('?') => Token::One,
            Item::Meta('[') => match set(&pattern[at..]) {
                Some((set, length)) => {
                    at += length;
                    Token::Set(set)
                }
                None => Token::Char('['),
            },
            Item::Literal(ch) | Item::Meta(ch) => Token::Char(*ch),
        });
    }
    tokens
}

/// The bracket expression after a `[` and how many items it spans, its `]`
/// included. A `]` first in the expression is a member, not its end.
fn set(pattern: &[Item]) -> Option<(Set, usize)> {
    let negated = matches!(pattern.first(), Some(Item::Literal('!' | '^')));
    let first = usize::from(negated);
    let mut ranges = Vec::new();
    let mut at = first;
    loop {
        let item = *pattern.get(at)?;
        if item == Item::Meta(']') && at > first {
            return Some((Set { negated, ranges }, at + 1));
        }
        let low = character(item);
        match (pattern.get(at + 1), pattern.get(at + 2)) {
            (Some(Item::Literal('-')), Some(high)) if *high != Item::Meta(']') => {
                ranges.push((low, character(*high)));
                at += 3;
            }
            _ => {
                ranges.push((low, low));
                at += 1;
            }
        }
    }
}

fn character(item: Item) -> char {
    match item {
        Item::Literal(ch) | Item::Meta(ch) => ch,
    }
}

/// Whether `name` matches a whole compiled component. Only the latest `*` is
/// ever retried, so the work stays proportional to pattern times name length.
fn matches(tokens: &[Token], name: &[char]) -> bool {
    let (mut token, mut at) = (0, 0);
    let mut retry: Option<(usize, usize)> = None;
    loop {
        match tokens.get(token) {
            Some(Token::Star) => {
                token += 1;
                retry = Some((token, at));
                continue;
            }
            Some(next) if name.get(at).is_some_and(|ch| next.accepts(*ch)) => {
                token += 1;
                at += 1;
                continue;
            }
            None if at == name.len() => return true,
            _ => {}
        }
        match &mut retry {
            Some((resume, from)) if *from < name.len() => {
                *from += 1;
                token = *resume;
                at = *from;
            }
            _ => return false,
        }
    }
}
