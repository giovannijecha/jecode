//! Parses one pattern of basic or extended syntax into a `Node` tree.

use super::{Boundary, Class, Item, Node, word};

pub(super) fn parse(pattern: &str, basic: bool) -> Result<Node, String> {
    let mut parser = Parser {
        chars: pattern.chars().collect(),
        at: 0,
        basic,
    };
    let root = parser.alternatives()?;
    if parser.at < parser.chars.len() {
        return Err(format!("unmatched ) in pattern: {pattern}"));
    }
    Ok(root)
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    /// Basic syntax writes the operators `( ) { } | + ?` with a backslash and
    /// treats them as literal without one; extended syntax is the reverse.
    basic: bool,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    /// Whether the operator `op` comes next in this syntax.
    fn operator(&self, op: char) -> bool {
        if self.basic && op != '*' {
            self.peek() == Some('\\') && self.chars.get(self.at + 1) == Some(&op)
        } else {
            self.peek() == Some(op)
        }
    }

    fn eat(&mut self, op: char) -> bool {
        let found = self.operator(op);
        if found {
            self.at += if self.basic && op != '*' { 2 } else { 1 };
        }
        found
    }

    fn alternatives(&mut self) -> Result<Node, String> {
        let mut options = vec![self.sequence()?];
        while self.eat('|') {
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
        while self.peek().is_some() && !self.operator('|') && !self.operator(')') {
            let atom = self.atom(nodes.is_empty())?;
            // A basic `*` after a leading `^` is a literal, not its repetition.
            nodes.push(if self.basic && matches!(atom, Node::Start) {
                atom
            } else {
                self.quantified(atom)?
            });
        }
        Ok(Node::Sequence(nodes))
    }

    fn quantified(&mut self, mut node: Node) -> Result<Node, String> {
        loop {
            let (min, max) = if self.eat('*') {
                (0, None)
            } else if self.eat('+') {
                (1, None)
            } else if self.eat('?') {
                (0, Some(1))
            } else if self.operator('{') {
                match self.bounds() {
                    Some(bounds) => bounds,
                    None => return Ok(node),
                }
            } else {
                return Ok(node);
            };
            // A lazy marker changes which match is reported, not whether one exists.
            if self.operator('?') {
                self.eat('?');
            }
            node = Node::Repeat(Box::new(node), min, max);
        }
    }

    /// `{m}`, `{m,}` or `{m,n}`; anything else leaves `{` literal.
    fn bounds(&mut self) -> Option<(usize, Option<usize>)> {
        let open = self.at + if self.basic { 2 } else { 1 };
        let close = self.chars[open..].iter().position(|ch| *ch == '}')? + open;
        let inside_end = close - usize::from(self.basic);
        if self.basic && self.chars[inside_end] != '\\' {
            return None;
        }
        let inside: String = self.chars[open..inside_end].iter().collect();
        let (min, max) = match inside.split_once(',') {
            None => (inside.parse().ok()?, Some(inside.parse().ok()?)),
            Some((min, "")) => (min.parse().ok()?, None),
            Some((min, max)) => (min.parse().ok()?, Some(max.parse().ok()?)),
        };
        self.at = close + 1;
        Some((min, max))
    }

    /// One atom. `first` says it starts its branch, the only place a basic `^`
    /// anchors. Basic `$` anchors only at the end of its branch.
    fn atom(&mut self, first: bool) -> Result<Node, String> {
        if self.eat('(') {
            let inner = self.alternatives()?;
            if !self.eat(')') {
                return Err("unmatched ( in pattern".into());
            }
            return Ok(inner);
        }
        // A leading extended repetition repeats nothing, as in GNU grep.
        if !self.basic && matches!(self.peek(), Some('*' | '+' | '?')) {
            return Ok(Node::Sequence(Vec::new()));
        }
        let ch = self.chars[self.at];
        self.at += 1;
        Ok(match ch {
            '.' => Node::Any,
            '^' if !self.basic || first => Node::Start,
            '$' if !self.basic
                || self.peek().is_none()
                || self.operator('|')
                || self.operator(')') =>
            {
                Node::End
            }
            '[' => Node::Class(self.class()?),
            '\\' => self.escape()?,
            _ => Node::Char(ch),
        })
    }

    fn escape(&mut self) -> Result<Node, String> {
        let Some(ch) = self.peek() else {
            return Ok(Node::Char('\\'));
        };
        self.at += 1;
        let named = |test: fn(char) -> bool, negated| {
            Node::Class(Class {
                negated: false,
                items: vec![Item::Named(test, negated)],
            })
        };
        Ok(match ch {
            '1'..='9' => {
                return Err(format!("back-references such as \\{ch} are unsupported"));
            }
            'd' | 'D' => named(|ch| ch.is_ascii_digit(), ch == 'D'),
            'w' | 'W' => named(word, ch == 'W'),
            's' | 'S' => named(char::is_whitespace, ch == 'S'),
            'b' | 'B' => Node::Boundary(Boundary::Word(ch == 'b')),
            '<' => Node::Boundary(Boundary::Start),
            '>' => Node::Boundary(Boundary::End),
            't' => Node::Char('\t'),
            'n' => Node::Char('\n'),
            _ => Node::Char(ch),
        })
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
                '\\' => match self.escape()? {
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

    /// `[:name:]` inside a bracket expression, with `at` on its first colon.
    fn posix(&mut self) -> Result<Item, String> {
        let rest = &self.chars[self.at + 1..];
        let end = rest
            .windows(2)
            .position(|pair| pair == [':', ']'])
            .ok_or("unterminated character class in pattern")?;
        let name: String = rest[..end].iter().collect();
        self.at += end + 3;
        let test: fn(char) -> bool = match name.as_str() {
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
