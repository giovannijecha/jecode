//! A small state-based regular-expression engine for `grep`: POSIX extended
//! syntax (`grep -E`), basic syntax converted to it, and the common GNU and
//! Perl shorthands (`\d`, `\w`, `\s`, `\b`, `\<`, `\>`). Matches are
//! leftmost-longest.

#[path = "regex/nfa.rs"]
mod nfa;

#[path = "regex/parser.rs"]
mod parser;

pub(super) struct Regex {
    machine: nfa::Machine,
    insensitive: bool,
}

#[cfg(test)]
#[path = "regex/tests.rs"]
mod tests;

#[derive(Clone)]
enum Node {
    Char(char),
    Any,
    Class(Class),
    Start,
    End,
    Boundary(Boundary),
    Sequence(Vec<Node>),
    Alternatives(Vec<Node>),
    Repeat(Box<Node>, usize, Option<usize>),
}

#[derive(Clone, Copy)]
enum Boundary {
    Word(bool),
    Start,
    End,
}

#[derive(Clone)]
struct Class {
    negated: bool,
    items: Vec<Item>,
}

#[derive(Clone)]
enum Item {
    Range(char, char),
    Named(fn(char) -> bool, bool),
}

impl Regex {
    /// Compiles extended syntax; see `extended` for basic patterns.
    pub(super) fn new(pattern: &str, insensitive: bool) -> Result<Self, String> {
        let mut parser = parser::Parser {
            chars: pattern.chars().collect(),
            at: 0,
        };
        let root = parser.alternatives()?;
        if parser.at < parser.chars.len() {
            return Err(format!("unmatched ) in pattern: {pattern}"));
        }
        Ok(Self {
            machine: nfa::Machine::compile(&root)?,
            insensitive,
        })
    }

    /// The first match in `text` as a character range.
    pub(super) fn find(&self, text: &[char], from: usize) -> Option<(usize, usize)> {
        self.machine.find(text, from, self.insensitive)
    }
}

fn word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Basic syntax writes the operators `( ) { } | + ?` with a backslash and
/// treats them as literal without one; extended syntax is the reverse.
pub(super) fn extended(pattern: &str) -> String {
    let mut result = String::new();
    let mut chars = pattern.chars().peekable();
    let mut bracket = false;
    while let Some(ch) = chars.next() {
        if bracket {
            result.push(ch);
            bracket = ch != ']' || result.ends_with("[]") || result.ends_with("[^]");
            continue;
        }
        match ch {
            '\\' => match chars.next() {
                Some(next @ ('(' | ')' | '{' | '}' | '|' | '+' | '?')) => result.push(next),
                Some(next) => {
                    result.push('\\');
                    result.push(next);
                }
                None => result.push('\\'),
            },
            '(' | ')' | '{' | '}' | '|' | '+' | '?' => {
                result.push('\\');
                result.push(ch);
            }
            '[' => {
                bracket = true;
                result.push(ch);
            }
            _ => result.push(ch),
        }
    }
    result
}
