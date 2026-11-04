//! Parses one pattern of basic or extended syntax into a `Node` tree.

use super::{Boundary, Class, Item, Node, word};

const UNMATCHED_BRACKET: &str = "unmatched [, [^, [:, [. or [= in pattern";
const RANGE_END: &str = "invalid range end in bracket expression";
const TOO_DEEP: &str = "regular expression nests too deeply";

/// Limits on nesting, because parsing and compiling recurse over it.
pub(super) const MAX_GROUPS: usize = 32;
pub(super) const MAX_STACKED: usize = 4;

pub(super) fn parse(pattern: &str, basic: bool) -> Result<Node, String> {
    let mut parser = Parser {
        chars: pattern.chars().collect(),
        at: 0,
        basic,
        groups: 0,
    };
    parser.alternatives()
}

/// One interval count; POSIX allows any implementation to stop here.
fn repetitions(digits: &str) -> Result<usize, String> {
    const RE_DUP_MAX: usize = 32_767;
    digits
        .parse()
        .ok()
        .filter(|count| *count <= RE_DUP_MAX)
        .ok_or("regular expression too big".into())
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    /// Basic syntax writes the operators `( ) { } | + ?` with a backslash and
    /// treats them as literal without one; extended syntax is the reverse.
    basic: bool,
    /// How many groups are open here.
    groups: usize,
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
        while self.peek().is_some() && !self.operator('|') {
            if self.operator(')') {
                if self.groups > 0 {
                    break;
                }
                // With no open group, an extended `)` is an ordinary character.
                if self.basic {
                    return Err("unmatched ) or \\) in pattern".into());
                }
            }
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
        let mut stacked = 0;
        loop {
            let (min, max) = if self.eat('*') {
                (0, None)
            } else if self.eat('+') {
                (1, None)
            } else if self.eat('?') {
                (0, Some(1))
            } else if self.operator('{') {
                match self.bounds()? {
                    Some(bounds) => bounds,
                    None => return Ok(node),
                }
            } else {
                return Ok(node);
            };
            stacked += 1;
            if stacked > MAX_STACKED {
                return Err(TOO_DEEP.into());
            }
            node = Node::Repeat(Box::new(node), min, max);
        }
    }

    /// An interval at `{`: `{m}`, `{m,}`, `{,n}`, `{,}` or `{m,n}`. Digits and
    /// commas that make none of these are an error. Other text after `{` is
    /// literal in extended syntax and an error in basic syntax.
    fn bounds(&mut self) -> Result<Option<(usize, Option<usize>)>, String> {
        const INVALID: &str = "invalid content of \\{\\}";
        let open = self.at + if self.basic { 2 } else { 1 };
        let rest = &self.chars[open..];
        let close: &[char] = if self.basic { &['\\', '}'] } else { &['}'] };
        let length = rest
            .iter()
            .take_while(|ch| ch.is_ascii_digit() || **ch == ',')
            .count();
        if !rest[length..].starts_with(close) {
            if !self.basic {
                return Ok(None);
            }
            let closed = rest.windows(2).any(|pair| pair == close);
            return Err(if closed { INVALID } else { "unmatched \\{" }.into());
        }
        let inside: String = rest[..length].iter().collect();
        let (min, max) = match inside.split_once(',') {
            None if inside.is_empty() => return Err(INVALID.into()),
            None => (repetitions(&inside)?, Some(repetitions(&inside)?)),
            Some((_, max)) if max.contains(',') => return Err(INVALID.into()),
            Some((min, max)) => (
                if min.is_empty() { 0 } else { repetitions(min)? },
                if max.is_empty() {
                    None
                } else {
                    Some(repetitions(max)?)
                },
            ),
        };
        if max.is_some_and(|max| max < min) {
            return Err(INVALID.into());
        }
        self.at = open + length + close.len();
        Ok(Some((min, max)))
    }

    /// One atom. `first` says it starts its branch, the only place a basic `^`
    /// anchors. Basic `$` anchors only at the end of its branch.
    fn atom(&mut self, first: bool) -> Result<Node, String> {
        if self.eat('(') {
            if self.groups == MAX_GROUPS {
                return Err(TOO_DEEP.into());
            }
            self.groups += 1;
            let inner = self.alternatives()?;
            self.groups -= 1;
            if !self.eat(')') {
                return Err("unmatched ( in pattern".into());
            }
            return Ok(inner);
        }
        // A leading extended repetition repeats nothing, as in GNU grep.
        if !self.basic {
            let at = self.at;
            if matches!(self.peek(), Some('*' | '+' | '?'))
                || self.peek() == Some('{') && self.bounds()?.is_some()
            {
                self.at = at;
                return Ok(Node::Sequence(Vec::new()));
            }
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
            return Err("trailing backslash in pattern".into());
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

    /// A bracket expression after its `[`. A backslash is an ordinary character
    /// here. A hyphen is one only first, last or as the end of a range.
    fn class(&mut self) -> Result<Class, String> {
        let negated = self.peek() == Some('^');
        self.at += usize::from(negated);
        let body = self.at;
        let mut items = Vec::new();
        let mut plain = true;
        loop {
            let first = self.at == body;
            let Some(ch) = self.peek() else {
                return Err(UNMATCHED_BRACKET.into());
            };
            self.at += 1;
            if ch == ']' && !first {
                break;
            }
            if ch == '-' && !first && self.peek().is_some_and(|next| next != ']') {
                return Err(RANGE_END.into());
            }
            let low = match (ch, self.peek()) {
                ('[', Some(':')) => {
                    items.push(self.posix()?);
                    plain = false;
                    continue;
                }
                ('[', Some('=')) => {
                    let member = self.collating('=')?;
                    items.push(Item::Range(member, member));
                    continue;
                }
                ('[', Some('.')) => self.collating('.')?,
                _ => ch,
            };
            if self.peek() == Some('-') && self.chars.get(self.at + 1).is_some_and(|ch| *ch != ']')
            {
                self.at += 1;
                let high = self.range_end()?;
                if high < low {
                    return Err(RANGE_END.into());
                }
                plain = false;
                items.push(Item::Range(low, high));
            } else {
                items.push(Item::Range(low, low));
            }
        }
        let members = &self.chars[body..self.at - 1];
        if plain
            && members.len() > 2
            && members.first() == Some(&':')
            && members.last() == Some(&':')
            && members.iter().any(|member| *member != ':')
        {
            return Err("character class syntax is [[:space:]], not [:space:]".into());
        }
        Ok(Class { negated, items })
    }

    fn range_end(&mut self) -> Result<char, String> {
        let ch = self.chars[self.at];
        self.at += 1;
        match (ch, self.peek()) {
            ('[', Some('.')) => self.collating('.'),
            ('[', Some(':' | '=')) => Err(RANGE_END.into()),
            _ => Ok(ch),
        }
    }

    /// `[.x.]` or `[=x=]` with `at` on the first `mark`. Only a single character
    /// can be named; these locales have no longer collating elements.
    fn collating(&mut self, mark: char) -> Result<char, String> {
        let rest = &self.chars[self.at + 1..];
        let end = rest
            .windows(2)
            .position(|pair| pair == [mark, ']'])
            .ok_or(UNMATCHED_BRACKET)?;
        let &[member] = &rest[..end] else {
            return Err("invalid collation character in bracket expression".into());
        };
        self.at += end + 3;
        Ok(member)
    }

    /// `[:name:]` inside a bracket expression, with `at` on its first colon.
    fn posix(&mut self) -> Result<Item, String> {
        let rest = &self.chars[self.at + 1..];
        let end = rest
            .windows(2)
            .position(|pair| pair == [':', ']'])
            .ok_or(UNMATCHED_BRACKET)?;
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
