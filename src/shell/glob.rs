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
    // A `[` with nothing to close it, as in the command `[ -f x ]`, is a
    // literal, and reading the directory for it would slow every test.
    let closes = |at: usize| pattern[at..].contains(&Item::Meta(']'));
    if !pattern.iter().enumerate().any(|(at, item)| match item {
        Item::Meta('*' | '?') => true,
        Item::Meta('[') => closes(at),
        _ => false,
    }) {
        return None;
    }
    let components: Vec<&[Item]> = pattern
        .split(|item| matches!(item, Item::Literal('/' | '\\')))
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
            // Names first: only a match in a middle component needs a stat.
            let mut names: Vec<String> = entries
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let shown =
                        !name.starts_with('.') || component.first() == Some(&Item::Literal('.'));
                    (shown
                        && matches(&tokens, &name.chars().collect::<Vec<_>>())
                        && (last || entry.path().is_dir()))
                    .then_some(name)
                })
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
    named: Vec<fn(char) -> bool>,
}

impl Token {
    fn accepts(&self, ch: char) -> bool {
        match self {
            Self::Star => false,
            Self::One => true,
            Self::Char(expected) => ch == *expected,
            Self::Set(set) => {
                let member = set
                    .ranges
                    .iter()
                    .any(|(low, high)| (*low..=*high).contains(&ch))
                    || set.named.iter().any(|test| test(ch));
                member != set.negated
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
    let mut named = Vec::new();
    let mut at = first;
    loop {
        let item = *pattern.get(at)?;
        if item == Item::Meta(']') && at > first {
            let set = Set {
                negated,
                ranges,
                named,
            };
            return Some((set, at + 1));
        }
        if item == Item::Meta('[')
            && pattern.get(at + 1) == Some(&Item::Literal(':'))
            && let Some((test, length)) = class_item(&pattern[at + 2..])
        {
            named.push(test);
            at += 2 + length;
            continue;
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

/// `name:]` after `[:`, with how many items it spans. An unknown name matches
/// nothing, so the pattern stays literal as in Bash.
fn class_item(pattern: &[Item]) -> Option<(fn(char) -> bool, usize)> {
    let end = pattern
        .windows(2)
        .position(|pair| pair == [Item::Literal(':'), Item::Meta(']')])?;
    let name: String = pattern[..end].iter().copied().map(character).collect();
    Some((named_class(&name).unwrap_or(|_| false), end + 2))
}

/// The test of a POSIX class such as `alpha`, for path and grep patterns.
pub(super) fn named_class(name: &str) -> Option<fn(char) -> bool> {
    Some(match name {
        "alpha" => char::is_alphabetic,
        "digit" => |ch| ch.is_ascii_digit(),
        "alnum" => char::is_alphanumeric,
        "space" => char::is_whitespace,
        "upper" => char::is_uppercase,
        "lower" => char::is_lowercase,
        "punct" => |ch| ch.is_ascii_punctuation(),
        "xdigit" => |ch| ch.is_ascii_hexdigit(),
        "blank" => |ch| ch == ' ' || ch == '\t',
        "cntrl" => char::is_control,
        "print" => |ch| !ch.is_control(),
        "graph" => |ch| !ch.is_control() && !ch.is_whitespace(),
        _ => return None,
    })
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
