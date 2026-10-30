#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Word(Word),
    Op(&'static str),
}

/// A token and its byte range in the script, so a command can be handed to a
/// child shell as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spanned {
    pub token: Token,
    pub start: usize,
    pub end: usize,
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

struct Cursor<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    position: usize,
}

impl Iterator for Cursor<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        let ch = self.chars.next()?;
        self.position += ch.len_utf8();
        Some(ch)
    }
}

impl Cursor<'_> {
    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }
}

#[derive(Default)]
struct Pending {
    word: Vec<Part>,
    literal: String,
    /// Unquoted characters of the word; quoted ones are recorded as NUL.
    unquoted: String,
    started: bool,
    /// The word so far is unquoted literal text, so `2>` names a descriptor.
    plain: bool,
    start: usize,
}

impl Pending {
    fn begin(&mut self, at: usize) {
        if !self.started {
            self.started = true;
            self.plain = true;
            self.start = at;
        }
    }

    fn quoted(&mut self, at: usize) {
        self.begin(at);
        self.plain = false;
    }

    fn flush(&mut self, tokens: &mut Vec<Spanned>, end: usize) -> Result<(), String> {
        if !self.started {
            return Ok(());
        }
        if brace_expansion(&self.unquoted) {
            return Err("unsupported shell syntax: brace expansion".into());
        }
        if !self.literal.is_empty() {
            self.word
                .push(Part::Literal(std::mem::take(&mut self.literal)));
        }
        tokens.push(Spanned {
            token: Token::Word(Word(std::mem::take(&mut self.word))),
            start: self.start,
            end,
        });
        self.unquoted.clear();
        self.started = false;
        Ok(())
    }

    /// Takes an unquoted `1` or `2` written directly before a redirection.
    fn descriptor(&mut self) -> Option<u8> {
        if !self.started || !self.plain || !self.word.is_empty() {
            return None;
        }
        let fd = match self.literal.as_str() {
            "1" => 1,
            "2" => 2,
            _ => return None,
        };
        self.literal.clear();
        self.unquoted.clear();
        self.started = false;
        Some(fd)
    }
}

pub fn tokenize(script: &str) -> Result<Vec<Spanned>, String> {
    let mut chars = Cursor {
        chars: script.chars().peekable(),
        position: 0,
    };
    let mut tokens = Vec::new();
    let mut pending = Pending::default();
    let mut quote = None;
    loop {
        let at = chars.position;
        let Some(ch) = chars.next() else { break };
        if quote != Some('\'')
            && ch == '$'
            && let Some(next) = chars.peek()
            && let Some(part) = variable(next, &mut chars)?
        {
            pending.quoted(at);
            if !pending.literal.is_empty() {
                pending
                    .word
                    .push(Part::Literal(std::mem::take(&mut pending.literal)));
            }
            pending.word.push(part);
            pending.unquoted.push('\0');
            continue;
        }
        if quote == Some('\'') {
            if ch == '\'' {
                quote = None;
            } else {
                pending.literal.push(ch);
                pending.unquoted.push('\0');
            }
            continue;
        }
        if ch == '\\' {
            match chars.next() {
                Some('\n') => {}
                Some(next) => {
                    pending.quoted(at);
                    if quote == Some('"') && !matches!(next, '$' | '"' | '\\') {
                        pending.literal.push('\\');
                    }
                    pending.literal.push(next);
                    pending.unquoted.push('\0');
                }
                None => return Err("trailing backslash".into()),
            }
            continue;
        }
        if ch == '"' {
            quote = if quote == Some('"') { None } else { Some('"') };
            pending.quoted(at);
            continue;
        }
        if quote.is_none() && ch == '\'' {
            quote = Some('\'');
            pending.quoted(at);
            continue;
        }
        if quote.is_none() {
            if ch == '`' {
                return Err(format!("unsupported shell syntax: {ch}"));
            }
            if ch == '#' && !pending.started {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
                separator(&mut tokens, chars.position);
                continue;
            }
            if ch.is_whitespace() {
                pending.flush(&mut tokens, at)?;
                if ch == '\n' {
                    separator(&mut tokens, at);
                }
                continue;
            }
            if ";&|<>()".contains(ch) {
                let fd = if ch == '>' {
                    pending.descriptor()
                } else {
                    None
                };
                pending.flush(&mut tokens, at)?;
                let op = operator(ch, fd, &mut chars)?;
                tokens.push(Spanned {
                    token: Token::Op(op),
                    start: at,
                    end: chars.position,
                });
                continue;
            }
            pending.unquoted.push(ch);
        } else {
            pending.unquoted.push('\0');
        }
        pending.begin(at);
        pending.literal.push(ch);
    }
    if quote.is_some() {
        return Err("unterminated quote".into());
    }
    pending.flush(&mut tokens, chars.position)?;
    Ok(tokens)
}

/// A newline or comment ends a command, except directly after `&&`, `||` or
/// `|`, where the command list continues on the next line.
fn separator(tokens: &mut Vec<Spanned>, at: usize) {
    if !matches!(
        tokens.last(),
        Some(Spanned {
            token: Token::Op("&&" | "||" | "|"),
            ..
        })
    ) {
        tokens.push(Spanned {
            token: Token::Op(";"),
            start: at,
            end: at,
        });
    }
}

fn operator(ch: char, fd: Option<u8>, chars: &mut Cursor<'_>) -> Result<&'static str, String> {
    let errors = fd == Some(2);
    Ok(match (ch, chars.peek()) {
        ('>', Some('&')) => {
            chars.next();
            if chars.next() != Some('2') || errors {
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
            if errors { "2>>" } else { ">>" }
        }
        ('>', _) if errors => "2>",
        (';', _) => ";",
        ('(', _) => "(",
        (')', _) => ")",
        ('|', _) => "|",
        ('>', _) => ">",
        ('<', _) => "<",
        ('&', _) => "&",
        _ => unreachable!(),
    })
}

fn variable(next: char, chars: &mut Cursor<'_>) -> Result<Option<Part>, String> {
    Ok(Some(match next {
        '?' => {
            chars.next();
            Part::Status
        }
        '$' => {
            chars.next();
            Part::Pid
        }
        '!' => {
            chars.next();
            Part::LastBackground
        }
        '@' => {
            chars.next();
            Part::AllArguments
        }
        '(' => {
            chars.next();
            Part::Command(substitution(chars)?)
        }
        '{' => {
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
            Part::Variable(name)
        }
        digit if digit.is_ascii_digit() => {
            chars.next();
            Part::Positional(digit.to_digit(10).unwrap() as usize)
        }
        start if start == '_' || start.is_ascii_alphabetic() => {
            let mut name = String::new();
            while chars
                .peek()
                .is_some_and(|c| c == '_' || c.is_ascii_alphanumeric())
            {
                name.push(chars.next().unwrap());
            }
            Part::Variable(name)
        }
        _ => return Ok(None),
    }))
}

fn substitution(chars: &mut Cursor<'_>) -> Result<String, String> {
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
                    return Ok(script);
                }
            }
        }
        script.push(c);
    }
    Err("unterminated command substitution".into())
}

/// Detects unquoted `{a,b}` or `{1..3}`, which Bash would expand. Literal
/// braces such as `{}` and `HEAD@{1}` stay ordinary word characters.
fn brace_expansion(unquoted: &str) -> bool {
    let mut open = None;
    for (index, ch) in unquoted.char_indices() {
        match ch {
            '{' => open = Some(index),
            '}' => {
                if let Some(start) = open.take() {
                    let inner = &unquoted[start + 1..index];
                    if inner.contains(',') || inner.contains("..") {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}
