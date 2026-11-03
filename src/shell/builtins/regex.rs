//! A small backtracking regular-expression engine for `grep`: POSIX extended
//! syntax (`grep -E`), basic syntax converted to it, and the common GNU and
//! Perl shorthands (`\d`, `\w`, `\s`, `\b`, `\<`, `\>`). Matches are
//! leftmost; alternatives and repetitions are tried in order, longest first.

pub(super) struct Regex {
    root: Node,
    insensitive: bool,
}

enum Node {
    Char(char),
    Any,
    Class(Class),
    Start,
    End,
    Boundary(bool),
    Sequence(Vec<Node>),
    Alternatives(Vec<Node>),
    Repeat(Box<Node>, usize, Option<usize>),
}

struct Class {
    negated: bool,
    items: Vec<Item>,
}

enum Item {
    Range(char, char),
    Named(fn(char) -> bool, bool),
}

impl Regex {
    /// Compiles extended syntax; see `extended` for basic patterns.
    pub(super) fn new(pattern: &str, insensitive: bool) -> Result<Self, String> {
        let mut parser = Parser {
            chars: pattern.chars().collect(),
            at: 0,
        };
        let root = parser.alternatives()?;
        if parser.at < parser.chars.len() {
            return Err(format!("unmatched ) in pattern: {pattern}"));
        }
        Ok(Self { root, insensitive })
    }

    /// The first match in `text` as a character range.
    pub(super) fn find(&self, text: &[char], from: usize) -> Option<(usize, usize)> {
        (from..=text.len()).find_map(|start| {
            let mut end = None;
            self.walk(&self.root, text, start, &mut |after| {
                end = Some(after);
                true
            });
            end.map(|end| (start, end))
        })
    }

    fn walk(
        &self,
        node: &Node,
        text: &[char],
        at: usize,
        next: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        let here = text.get(at).copied();
        match node {
            Node::Char(expected) => here.is_some_and(|ch| self.same(ch, *expected)) && next(at + 1),
            Node::Any => here.is_some() && next(at + 1),
            Node::Class(class) => here.is_some_and(|ch| self.member(class, ch)) && next(at + 1),
            Node::Start => at == 0 && next(at),
            Node::End => at == text.len() && next(at),
            Node::Boundary(wanted) => {
                let before = at > 0 && word(text[at - 1]);
                let after = here.is_some_and(word);
                (before != after) == *wanted && next(at)
            }
            Node::Sequence(nodes) => self.sequence(nodes, text, at, next),
            Node::Alternatives(options) => options
                .iter()
                .any(|option| self.walk(option, text, at, next)),
            Node::Repeat(inner, min, max) => self.repeat(inner, (*min, *max), 0, text, at, next),
        }
    }

    fn sequence(
        &self,
        nodes: &[Node],
        text: &[char],
        at: usize,
        next: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        match nodes.split_first() {
            None => next(at),
            Some((first, rest)) => self.walk(first, text, at, &mut |after| {
                self.sequence(rest, text, after, next)
            }),
        }
    }

    fn repeat(
        &self,
        inner: &Node,
        (min, max): (usize, Option<usize>),
        count: usize,
        text: &[char],
        at: usize,
        next: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        // An empty iteration counts only toward the minimum, so `(a*)*` ends.
        if max.is_none_or(|max| count < max)
            && self.walk(inner, text, at, &mut |after| {
                (after != at || count < min)
                    && self.repeat(inner, (min, max), count + 1, text, after, next)
            })
        {
            return true;
        }
        count >= min && next(at)
    }

    fn same(&self, actual: char, expected: char) -> bool {
        actual == expected
            || (self.insensitive && actual.to_lowercase().eq(expected.to_lowercase()))
    }

    fn member(&self, class: &Class, ch: char) -> bool {
        let test = |ch: char| {
            class.items.iter().any(|item| match item {
                Item::Range(low, high) => (*low..=*high).contains(&ch),
                Item::Named(test, negated) => test(ch) != *negated,
            })
        };
        let found = test(ch)
            || (self.insensitive && (ch.to_lowercase().any(test) || ch.to_uppercase().any(test)));
        found != class.negated
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

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn alternatives(&mut self) -> Result<Node, String> {
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
            'b' | 'B' => Node::Boundary(ch == 'b'),
            '<' | '>' => Node::Boundary(true),
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
