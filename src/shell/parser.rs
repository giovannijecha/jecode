use super::lexer::{Part, Spanned, Token, Word};

#[derive(Default)]
pub(super) struct Simple {
    pub words: Vec<Word>,
    pub redirects: Vec<Redirect>,
}

pub(super) struct Redirect {
    pub stream: u8,
    pub mode: &'static str,
    pub target: Word,
}

pub(super) enum Command {
    Simple(Simple),
    Compound(Compound, Vec<Redirect>),
}

pub(super) enum Compound {
    For {
        name: Word,
        values: Vec<Word>,
        body: List,
    },
    CountedFor {
        init: Word,
        name: Word,
        limit: Word,
        increment: Word,
        body: List,
    },
    Loop {
        until: bool,
        condition: List,
        body: List,
    },
    If {
        branches: Vec<(List, List)>,
        otherwise: Option<List>,
    },
    Subshell(List),
}

pub(super) type Pipeline = Vec<Command>;

pub(super) struct AndOr {
    pub first: Pipeline,
    pub rest: Vec<(&'static str, Pipeline)>,
}

/// One command list entry. A background entry keeps its source text, which a
/// child shell expands and runs.
pub(super) struct Item {
    pub and_or: AndOr,
    pub background: Option<String>,
}

pub(super) type List = Vec<Item>;

const CLOSERS: [&str; 6] = ["do", "done", "then", "else", "elif", "fi"];
pub(super) const COUNTED_FOR: &str = "for: expected 'for ((NAME=START; NAME<LIMIT; NAME++)); do'";

pub(super) fn parse(script: &str, tokens: &[Spanned]) -> Result<List, String> {
    let mut parser = Parser {
        script,
        tokens,
        index: 0,
    };
    let list = parser.list()?;
    match parser.peek() {
        None => Ok(list),
        Some(token) => Err(unexpected(token)),
    }
}

struct Parser<'a> {
    script: &'a str,
    tokens: &'a [Spanned],
    index: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index).map(|spanned| &spanned.token)
    }

    fn op_is(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Token::Op(found)) if *found == op)
    }

    fn word_is(&self, keyword: &str) -> bool {
        self.peek().is_some_and(|token| literal(token, keyword))
    }

    fn eat_op(&mut self, op: &str) -> bool {
        let found = self.op_is(op);
        self.index += usize::from(found);
        found
    }

    fn eat_word(&mut self, keyword: &str) -> bool {
        let found = self.word_is(keyword);
        self.index += usize::from(found);
        found
    }

    fn take_word(&mut self) -> Option<Word> {
        let Some(Token::Word(word)) = self.peek() else {
            return None;
        };
        let word = word.clone();
        self.index += 1;
        Some(word)
    }

    fn skip_separators(&mut self) {
        while self.eat_op(";") {}
    }

    fn list(&mut self) -> Result<List, String> {
        let mut items = Vec::new();
        loop {
            self.skip_separators();
            match self.peek() {
                None | Some(Token::Op(")")) => break,
                Some(token) if CLOSERS.iter().any(|closer| literal(token, closer)) => break,
                _ => {}
            }
            let start = self.tokens[self.index].start;
            let and_or = self.and_or()?;
            let end = self.tokens[self.index - 1].end;
            let background = self.eat_op("&");
            if !background && !self.eat_op(";") {
                match self.peek() {
                    None | Some(Token::Op(")")) => {}
                    Some(token) => return Err(unexpected(token)),
                }
            }
            items.push(Item {
                and_or,
                background: background.then(|| self.script[start..end].to_owned()),
            });
        }
        Ok(items)
    }

    fn and_or(&mut self) -> Result<AndOr, String> {
        let first = self.pipeline()?;
        let mut rest = Vec::new();
        while let Some(Token::Op(op @ ("&&" | "||"))) = self.peek() {
            let op = *op;
            self.index += 1;
            rest.push((op, self.pipeline()?));
        }
        Ok(AndOr { first, rest })
    }

    fn pipeline(&mut self) -> Result<Pipeline, String> {
        let mut stages = vec![self.command()?];
        while self.eat_op("|") {
            stages.push(self.command()?);
        }
        Ok(stages)
    }

    fn command(&mut self) -> Result<Command, String> {
        let compound = match self.peek() {
            None => return Err("incomplete command list".into()),
            Some(Token::Op("(")) => {
                self.index += 1;
                let list = self.list()?;
                if !self.eat_op(")") {
                    return Err("missing subshell ')'".into());
                }
                Compound::Subshell(list)
            }
            Some(token @ Token::Op(op)) if !redirect_op(op) => return Err(unexpected(token)),
            Some(token) if literal(token, "for") => self.for_loop()?,
            Some(token) if literal(token, "while") || literal(token, "until") => {
                let until = literal(token, "until");
                self.index += 1;
                let condition = self.condition("loop")?;
                if !self.eat_word("do") {
                    return Err("loop: expected do".into());
                }
                let body = self.list()?;
                if !self.eat_word("done") {
                    return Err("loop: missing done".into());
                }
                Compound::Loop {
                    until,
                    condition,
                    body,
                }
            }
            Some(token) if literal(token, "if") => self.if_command()?,
            Some(token) if CLOSERS.iter().any(|closer| literal(token, closer)) => {
                return Err(unexpected(token));
            }
            Some(token) if literal(token, "{") || literal(token, "}") => {
                return Err("unsupported shell syntax: brace group".into());
            }
            _ => return self.simple().map(Command::Simple),
        };
        let mut redirects = Vec::new();
        while let Some(Token::Op(op)) = self.peek()
            && redirect_op(op)
        {
            redirects.push(self.redirect()?);
        }
        if let Some(token @ Token::Word(_)) = self.peek() {
            return Err(unexpected(token));
        }
        Ok(Command::Compound(compound, redirects))
    }

    fn condition(&mut self, name: &str) -> Result<List, String> {
        let condition = self.list()?;
        if condition.is_empty() {
            return Err(format!("{name}: expected a condition"));
        }
        Ok(condition)
    }

    fn simple(&mut self) -> Result<Simple, String> {
        let mut simple = Simple::default();
        loop {
            match self.peek() {
                Some(Token::Word(word)) => {
                    simple.words.push(word.clone());
                    self.index += 1;
                }
                Some(Token::Op(op)) if redirect_op(op) => {
                    simple.redirects.push(self.redirect()?);
                }
                _ => return Ok(simple),
            }
        }
    }

    fn redirect(&mut self) -> Result<Redirect, String> {
        let Some(Token::Op(op)) = self.peek() else {
            unreachable!()
        };
        let op = *op;
        self.index += 1;
        let (stream, mode) = match op {
            ">&2" => {
                return Ok(Redirect {
                    stream: 1,
                    mode: ">&2",
                    target: Word(Vec::new()),
                });
            }
            "2>" => (2, ">"),
            "2>>" => (2, ">>"),
            mode => (if mode == "<" { 0 } else { 1 }, mode),
        };
        let target = self.take_word().ok_or("redirection needs a path")?;
        Ok(Redirect {
            stream,
            mode,
            target,
        })
    }

    fn for_loop(&mut self) -> Result<Compound, String> {
        self.index += 1;
        if self.op_is("(") {
            return self.counted_for();
        }
        let header = "for: expected 'for NAME in WORDS; do'";
        let name = self.take_word().ok_or(header)?;
        if !self.eat_word("in") {
            return Err(header.into());
        }
        let mut values = Vec::new();
        while let Some(value) = self.take_word() {
            values.push(value);
        }
        self.skip_separators();
        if !self.eat_word("do") {
            return Err(header.into());
        }
        let body = self.list()?;
        if !self.eat_word("done") {
            return Err("for: missing done".into());
        }
        Ok(Compound::For { name, values, body })
    }

    fn counted_for(&mut self) -> Result<Compound, String> {
        let mut header = || -> Option<(Word, Word, Word, Word)> {
            (self.eat_op("(") && self.eat_op("(")).then_some(())?;
            let init = self.take_word()?;
            self.eat_op(";").then_some(())?;
            let name = self.take_word()?;
            self.eat_op("<").then_some(())?;
            let limit = self.take_word()?;
            self.eat_op(";").then_some(())?;
            let increment = self.take_word()?;
            (self.eat_op(")") && self.eat_op(")")).then_some(())?;
            self.skip_separators();
            self.eat_word("do")
                .then_some((init, name, limit, increment))
        };
        let (init, name, limit, increment) = header().ok_or(COUNTED_FOR)?;
        let body = self.list()?;
        if !self.eat_word("done") {
            return Err("for: missing done".into());
        }
        Ok(Compound::CountedFor {
            init,
            name,
            limit,
            increment,
            body,
        })
    }

    fn if_command(&mut self) -> Result<Compound, String> {
        self.index += 1;
        let mut branches = Vec::new();
        loop {
            let condition = self.condition("if")?;
            if !self.eat_word("then") {
                return Err("if: expected then".into());
            }
            branches.push((condition, self.list()?));
            if !self.eat_word("elif") {
                break;
            }
        }
        let otherwise = if self.eat_word("else") {
            Some(self.list()?)
        } else {
            None
        };
        if !self.eat_word("fi") {
            return Err("if: missing fi".into());
        }
        Ok(Compound::If {
            branches,
            otherwise,
        })
    }
}

fn redirect_op(op: &str) -> bool {
    matches!(op, ">" | ">>" | "<" | "2>" | "2>>" | ">&2")
}

fn unexpected(token: &Token) -> String {
    match token {
        Token::Op(op) => format!("unexpected '{op}'"),
        Token::Word(Word(parts)) => match parts.as_slice() {
            [Part::Literal(text)] => format!("unexpected '{text}'"),
            _ => "unexpected word".into(),
        },
    }
}

pub(super) fn literal(token: &Token, expected: &str) -> bool {
    matches!(token, Token::Word(Word(parts)) if parts.as_slice() == [Part::Literal(expected.to_owned())])
}
