//! A small state-based regular-expression engine for `grep`: POSIX basic and
//! extended syntax, fixed strings, and the common GNU and Perl shorthands
//! (`\d`, `\w`, `\s`, `\b`, `\<`, `\>`). Matches are leftmost-longest.

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Syntax {
    Basic,
    Extended,
    Fixed,
}

/// Where a match must begin and end: anywhere, on word edges (`-w`) or across
/// the whole line (`-x`).
#[derive(Clone, Copy)]
pub(super) enum Edges {
    Any,
    Word,
    Line,
}

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
    /// The edges of a `-w` match, which may itself start or end with a
    /// non-word character, unlike `\b`.
    NoWordBefore,
    NoWordAfter,
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
    /// Compiles patterns of which any may match. Each is parsed on its own, so
    /// its anchors and groups cannot reach into another.
    pub(super) fn new(
        patterns: &[&str],
        syntax: Syntax,
        edges: Edges,
        insensitive: bool,
    ) -> Result<Self, String> {
        let mut options = patterns
            .iter()
            .map(|pattern| match syntax {
                Syntax::Fixed => Ok(Node::Sequence(pattern.chars().map(Node::Char).collect())),
                _ => parser::parse(pattern, syntax == Syntax::Basic),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let root = if options.len() == 1 {
            options.remove(0)
        } else {
            Node::Alternatives(options)
        };
        let root = match edges {
            Edges::Any => root,
            Edges::Line => Node::Sequence(vec![Node::Start, root, Node::End]),
            Edges::Word => Node::Sequence(vec![
                Node::Boundary(Boundary::NoWordBefore),
                root,
                Node::Boundary(Boundary::NoWordAfter),
            ]),
        };
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
