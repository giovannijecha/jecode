#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Word(Word),
    Op(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word(pub Vec<Part>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Literal(String),
    Variable(String),
    Status,
    Positional(usize),
    AllArguments,
    Pid,
    LastBackground,
    Command(String),
}

pub fn tokenize(script: &str) -> Result<Vec<Token>, String> {
    let mut chars = script.chars().peekable();
    let mut tokens = Vec::new();
    let mut word = Vec::new();
    let mut literal = String::new();
    let mut started = false;
    let mut quote = None;
    while let Some(ch) = chars.next() {
        if quote != Some('\'')
            && ch == '$'
            && let Some(next) = chars.peek().copied()
        {
            let variable = if next == '?' {
                chars.next();
                Some(Part::Status)
            } else if next == '$' {
                chars.next();
                Some(Part::Pid)
            } else if next == '!' {
                chars.next();
                Some(Part::LastBackground)
            } else if next == '(' {
                chars.next();
                let mut script = String::new();
                let mut depth = 1;
                let mut inner_quote = None;
                for c in chars.by_ref() {
                    if c == '\'' || c == '"' {
                        if inner_quote == Some(c) {
                            inner_quote = None;
                        } else if inner_quote.is_none() {
                            inner_quote = Some(c);
                        }
                    }
                    if inner_quote.is_none() {
                        if c == '(' {
                            depth += 1;
                        }
                        if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                    }
                    script.push(c);
                }
                if depth != 0 {
                    return Err("unterminated command substitution".into());
                }
                Some(Part::Command(script))
            } else if next == '@' {
                chars.next();
                Some(Part::AllArguments)
            } else if next.is_ascii_digit() {
                Some(Part::Positional(
                    chars.next().unwrap().to_digit(10).unwrap() as usize,
                ))
            } else if next == '{' {
                chars.next();
                let mut name = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    name.push(c);
                }
                if name.is_empty() || !closed {
                    return Err("empty or unterminated variable name".into());
                }
                Some(Part::Variable(name))
            } else if next == '_' || next.is_ascii_alphabetic() {
                let mut name = String::new();
                while chars
                    .peek()
                    .is_some_and(|c| *c == '_' || c.is_ascii_alphanumeric())
                {
                    name.push(chars.next().unwrap());
                }
                Some(Part::Variable(name))
            } else {
                None
            };
            if let Some(variable) = variable {
                if !literal.is_empty() {
                    word.push(Part::Literal(std::mem::take(&mut literal)));
                }
                word.push(variable);
                started = true;
                continue;
            }
        }
        if quote == Some('\'') {
            if ch == '\'' {
                quote = None;
            } else {
                literal.push(ch);
            }
            continue;
        }
        if ch == '\\' {
            if let Some(next) = chars.next() {
                if next != '\n' {
                    if quote == Some('"') && !matches!(next, '$' | '"' | '\\') {
                        literal.push('\\');
                    }
                    literal.push(next);
                }
            } else {
                return Err("trailing backslash".into());
            }
            started = true;
            continue;
        }
        if ch == '"' {
            quote = if quote == Some('"') { None } else { Some('"') };
            started = true;
            continue;
        }
        if quote.is_none() && ch == '\'' {
            quote = Some('\'');
            started = true;
            continue;
        }
        if quote.is_none() {
            if matches!(ch, '`' | '{' | '}') {
                return Err(format!("unsupported shell syntax: {ch}"));
            }
            if ch == '#' && !started {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
                flush(&mut tokens, &mut word, &mut literal, &mut started);
                tokens.push(Token::Op(";"));
                continue;
            }
            if ch.is_whitespace() {
                flush(&mut tokens, &mut word, &mut literal, &mut started);
                if ch == '\n' {
                    tokens.push(Token::Op(";"));
                }
                continue;
            }
            if ";&|<>()".contains(ch) {
                flush(&mut tokens, &mut word, &mut literal, &mut started);
                let op = match (ch, chars.peek().copied()) {
                    ('>', Some('&')) => {
                        chars.next();
                        if chars.next() != Some('2') {
                            return Err("only >&2 descriptor redirection is supported".into());
                        }
                        ">&2"
                    }
                    ('&', Some('&')) => {
                        chars.next();
                        "&&"
                    }
                    ('|', Some('|')) => {
                        chars.next();
                        "||"
                    }
                    ('>', Some('>')) => {
                        chars.next();
                        ">>"
                    }
                    (';', _) => ";",
                    ('(', _) => "(",
                    (')', _) => ")",
                    ('|', _) => "|",
                    ('>', _) => ">",
                    ('<', _) => "<",
                    ('&', _) => "&",
                    _ => unreachable!(),
                };
                tokens.push(Token::Op(op));
                continue;
            }
        }
        literal.push(ch);
        started = true;
    }
    if quote.is_some() {
        return Err("unterminated quote".into());
    }
    flush(&mut tokens, &mut word, &mut literal, &mut started);
    Ok(tokens)
}

fn flush(tokens: &mut Vec<Token>, word: &mut Vec<Part>, literal: &mut String, started: &mut bool) {
    if !*started {
        return;
    }
    if !literal.is_empty() {
        word.push(Part::Literal(std::mem::take(literal)));
    }
    tokens.push(Token::Word(Word(std::mem::take(word))));
    *started = false;
}
