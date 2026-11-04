//! Parses extended pattern syntax into a `Node` tree.

use super::{Boundary, Class, Item, Node, word};

pub(super) struct Parser {
    pub(super) chars: Vec<char>,
    pub(super) at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    pub(super) fn alternatives(&mut self) -> Result<Node, String> {
        let mut options = vec![self.sequence()?];
        while self.peek() == Some('|') {
            self.at += 1;
            options.push(self.sequence()?);
        }
        Ok(if options.len() == 1 {
            options.pop().unwrap()
        } else {
            Node::Alternatives(options)
        })
    }

    fn sequence(&mut self) -> Result<Node, String> {
        let mut nodes = Vec::new();
        while let Some(ch) = self.peek() {
            if ch == '|' || ch == ')' {
                break;
            }
            let atom = self.atom()?;
            nodes.push(self.quantified(atom)?);
        }
        Ok(Node::Sequence(nodes))
    }

    fn quantified(&mut self, mut node: Node) -> Result<Node, String> {
        loop {
            let (min, max) = match self.peek() {
                Some('{') => match self.bounds() {
                    Some(bounds) => bounds,
                    None => return Ok(node),
                },
                Some(ch @ ('*' | '+' | '?')) => {
                    self.at += 1;
                    match ch {
                        '*' => (0, None),
                        '+' => (1, None),
                        _ => (0, Some(1)),
                    }
                }
                _ => return Ok(node),
            };
            // A lazy marker changes which match is reported, not whether one exists.
            if self.peek() == Some('?') {
                self.at += 1;
            }
            node = Node::Repeat(Box::new(node), min, max);
        }
    }

    /// `{m}`, `{m,}` or `{m,n}`; anything else leaves `{` literal.
    fn bounds(&mut self) -> Option<(usize, Option<usize>)> {
        let close = self.chars[self.at..].iter().position(|ch| *ch == '}')? + self.at;
        let inside: String = self.chars[self.at + 1..close].iter().collect();
        let (min, max) = match inside.split_once(',') {
            None => (inside.parse().ok()?, Some(inside.parse().ok()?)),
            Some((min, "")) => (min.parse().ok()?, None),
            Some((min, max)) => (min.parse().ok()?, Some(max.parse().ok()?)),
        };
        self.at = close + 1;
        Some((min, max))
    }

    fn atom(&mut self) -> Result<Node, String> {
        let ch = self.chars[self.at];
        self.at += 1;
        Ok(match ch {
            '(' => {
                let inner = self.alternatives()?;
                if self.peek() != Some(')') {
                    return Err("unmatched ( in pattern".into());
                }
                self.at += 1;
                inner
            }
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '[' => Node::Class(self.class()?),
            '\\' => self.escape(),
            // A quantifier with nothing to repeat is an ordinary character.
            _ => Node::Char(ch),
        })
    }

    fn escape(&mut self) -> Node {
        let Some(ch) = self.peek() else {
            return Node::Char('\\');
        };
        self.at += 1;
        let named = |test: fn(char) -> bool, negated| {
            Node::Class(Class {
                negated: false,
                items: vec![Item::Named(test, negated)],
            })
        };
        match ch {
            'd' | 'D' => named(|ch| ch.is_ascii_digit(), ch == 'D'),
            'w' | 'W' => named(word, ch == 'W'),
            's' | 'S' => named(char::is_whitespace, ch == 'S'),
            'b' | 'B' => Node::Boundary(Boundary::Word(ch == 'b')),
            '<' => Node::Boundary(Boundary::Start),
            '>' => Node::Boundary(Boundary::End),
            't' => Node::Char('\t'),
            'n' => Node::Char('\n'),
            _ => Node::Char(ch),
        }
    }

    fn class(&mut self) -> Result<Class, String> {
        let negated = self.peek() == Some('^');
        self.at += usize::from(negated);
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let Some(ch) = self.peek() else {
                return Err("unmatched [ in pattern".into());
            };
            self.at += 1;
            if ch == ']' && !first {
                return Ok(Class { negated, items });
            }
            first = false;
            let low = match ch {
                '[' if self.peek() == Some(':') => {
                    items.push(self.posix()?);
                    continue;
                }
                '\\' => match self.escape() {
                    Node::Class(class) => {
                        items.extend(class.items);
                        continue;
                    }
                    Node::Char(ch) => ch,
                    _ => 'b',
                },
                _ => ch,
            };
            if self.peek() == Some('-') && self.chars.get(self.at + 1).is_some_and(|ch| *ch != ']')
            {
                let high = self.chars[self.at + 1];
                self.at += 2;
                items.push(Item::Range(low, high));
            } else {
                items.push(Item::Range(low, low));
            }
        }
    }

    /// `[:name:]` inside a bracket expression.
    fn posix(&mut self) -> Result<Item, String> {
        let rest: String = self.chars[self.at..].iter().collect();
        let end = rest
            .find(":]")
            .ok_or("unterminated character class in pattern")?;
        let name = &rest[1..end];
        self.at += end + 2;
        let test: fn(char) -> bool = match name {
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
            _ => return Err(format!("unknown character class [:{name}:]")),
        };
        Ok(Item::Named(test, false))
    }
}
