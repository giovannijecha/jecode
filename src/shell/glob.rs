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
                .filter(|name| matches(component, &name.chars().collect::<Vec<_>>()))
                .collect();
            names.sort();
            next.extend(names.iter().map(|name| joined(name)));
        }
        prefixes = next;
    }
    let found: Vec<String> = prefixes
        .into_iter()
        .filter(|path| Path::new(path).is_absolute() || cwd.join(path).exists())
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
    matches(&pattern, &name.chars().collect::<Vec<_>>())
}

/// Whether `name` matches one path component of the pattern.
fn matches(pattern: &[Item], name: &[char]) -> bool {
    let Some((first, rest)) = pattern.split_first() else {
        return name.is_empty();
    };
    match first {
        Item::Meta('*') => (0..=name.len()).any(|skip| matches(rest, &name[skip..])),
        Item::Meta('?') => !name.is_empty() && matches(rest, &name[1..]),
        Item::Meta('[') => match class(rest) {
            Some((accepts, after)) => {
                !name.is_empty() && accepts(name[0]) && matches(after, &name[1..])
            }
            None => name.first() == Some(&'[') && matches(rest, &name[1..]),
        },
        Item::Literal(ch) | Item::Meta(ch) => name.first() == Some(ch) && matches(rest, &name[1..]),
    }
}

/// A bracket expression after `[`: its test and the pattern after `]`.
fn class(pattern: &[Item]) -> Option<(impl Fn(char) -> bool + '_, &[Item])> {
    let negated = matches!(
        pattern.first(),
        Some(Item::Literal('!' | '^') | Item::Meta('!' | '^'))
    );
    let body = &pattern[usize::from(negated)..];
    // A `]` first in the expression is a member, not its end.
    let end = body
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, item)| **item == Item::Meta(']'))
        .map(|(index, _)| index)?;
    let members: Vec<char> = body[..end]
        .iter()
        .map(|item| match item {
            Item::Literal(ch) | Item::Meta(ch) => *ch,
        })
        .collect();
    let accepts = move |ch: char| {
        let mut index = 0;
        let mut found = false;
        while index < members.len() {
            if index + 2 < members.len() && members[index + 1] == '-' {
                found |= (members[index]..=members[index + 2]).contains(&ch);
                index += 3;
            } else {
                found |= members[index] == ch;
                index += 1;
            }
        }
        found != negated
    };
    Some((accepts, &body[end + 1..]))
}
