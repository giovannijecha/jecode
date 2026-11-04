#[path = "heredoc.rs"]
mod heredoc;

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
    /// Original here-document body and closing line, if this is its target.
    pub body: Option<(usize, usize)>,
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
    /// An unquoted `*`, `?`, `[` or `]`, for pathname expansion.
    Glob(char),
}

struct Cursor<'a> {
    source: &'a str,
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

impl<'a> Cursor<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.chars().peekable(),
            position: 0,
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn rest(&self) -> &'a str {
        &self.source[self.position..]
    }

    /// Moves forward by `length` bytes, which must end on a character boundary.
    fn advance(&mut self, length: usize) {
        let target = self.position + length;
        while self.position < target && self.next().is_some() {}
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
            body: None,
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
    let mut chars = Cursor::new(script);
    let mut tokens = Vec::new();
    let mut pending = Pending::default();
    let mut quote = None;
    let mut documents = Vec::new();
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
        if ch == '`' {
            return Err(format!("unsupported shell syntax: {ch}"));
        }
        if quote.is_none() {
            if ch == '#' && !pending.started {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
                separator(&mut tokens, chars.position);
                heredoc::bodies(&mut chars, &mut tokens, &mut documents)?;
                continue;
            }
            if ch.is_whitespace() {
                pending.flush(&mut tokens, at)?;
                if ch == '\n' {
                    separator(&mut tokens, at);
                    heredoc::bodies(&mut chars, &mut tokens, &mut documents)?;
                }
                continue;
            }
            if ch == '<' && chars.peek() == Some('<') {
                pending.flush(&mut tokens, at)?;
                chars.next();
                heredoc::start(&mut chars, &mut tokens, at, &mut documents)?;
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
                    body: None,
                });
                continue;
            }
            pending.unquoted.push(ch);
            if matches!(ch, '*' | '?' | '[' | ']') {
                pending.begin(at);
                if !pending.literal.is_empty() {
                    pending
                        .word
                        .push(Part::Literal(std::mem::take(&mut pending.literal)));
                }
                pending.word.push(Part::Glob(ch));
                continue;
            }
        } else {
            pending.unquoted.push('\0');
        }
        pending.begin(at);
        pending.literal.push(ch);
    }
    if quote.is_some() {
        return Err("unterminated quote".into());
    }
    if let Some(document) = documents.first() {
        return Err(format!(
            "here-document needs its body on the following lines, ending with a {} line",
            document.delimiter()
        ));
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
            body: None,
        });
    }
}

fn operator(ch: char, fd: Option<u8>, chars: &mut Cursor<'_>) -> Result<&'static str, String> {
    let errors = fd == Some(2);
    Ok(match (ch, chars.peek()) {
        ('>', Some('&')) => {
            chars.next();
            match chars.next() {
                Some('2') if !errors => ">&2",
                Some('1') if errors => "2>&1",
                _ => {
                    return Err("only 1>&2 and 2>&1 descriptor redirections are supported".into());
                }
            }
        }
        ('&', Some('&')) => {
            chars.next();
            "&&"
        }
        ('&', Some('>')) => {
            chars.next();
            if chars.peek() == Some('>') {
                chars.next();
                "&>>"
            } else {
                "&>"
            }
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
            let script = substitution(chars)?;
            let tokens = tokenize(&script)?;
            super::parser::parse(&script, &tokens)?;
            Part::Command(script)
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

/// The source text up to the `)` closing a `$(`. Quotes, comments and
/// here-document bodies may hold an unbalanced `)` or quote, so they are skipped
/// as the substitution's own interpreter will read them.
fn substitution(chars: &mut Cursor<'_>) -> Result<String, String> {
    let start = chars.position;
    let mut depth = 1;
    let mut inner_quote = None;
    let mut word_start = true;
    let mut documents = Vec::new();
    loop {
        let at = chars.position;
        let Some(c) = chars.next() else {
            return Err("unterminated command substitution".into());
        };
        let starts_word =
            std::mem::replace(&mut word_start, c.is_whitespace() || ";&|(".contains(c));
        if c == '\\' && inner_quote != Some('\'') {
            chars.next();
            continue;
        }
        if c == '\'' || c == '"' {
            if inner_quote == Some(c) {
                inner_quote = None;
            } else if inner_quote.is_none() {
                inner_quote = Some(c);
            }
            continue;
        }
        if inner_quote.is_some() {
            continue;
        }
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(chars.source[start..at].to_owned());
                }
            }
            '#' if starts_word => {
                while chars.peek().is_some_and(|c| c != '\n') {
                    chars.next();
                }
            }
            '<' if chars.peek() == Some('<') => {
                chars.next();
                if chars.peek() == Some('<') {
                    chars.next();
                } else {
                    let (delimiter, _, strip_tabs) = heredoc::header(chars)?;
                    documents.push((delimiter, strip_tabs));
                }
            }
            '\n' => heredoc::skip_bodies(chars, &mut documents)?,
            _ => {}
        }
    }
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
