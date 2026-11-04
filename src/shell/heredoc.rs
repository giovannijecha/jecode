//! Here-documents and here-strings. `<<WORD` and `<<-WORD` take the lines
//! after the command, up to a line holding only WORD, as standard input. A
//! quoted WORD keeps the text literal; otherwise `$` expansions and `\`
//! escapes apply, as in POSIX shells. `<<< word` passes one word and a newline.

use super::{Cursor, Part, Spanned, Token, Word, variable};

pub(super) struct Document {
    delimiter: String,
    quoted: bool,
    strip_tabs: bool,
    token: usize,
}

impl Document {
    pub(super) fn delimiter(&self) -> &str {
        &self.delimiter
    }
}

/// Reads what follows `<<`: a here-string operator, or a delimiter whose
/// body arrives after the next newline. The body's word is a placeholder
/// until then.
pub(super) fn start(
    chars: &mut Cursor<'_>,
    tokens: &mut Vec<Spanned>,
    at: usize,
    documents: &mut Vec<Document>,
) -> Result<(), String> {
    if chars.peek() == Some('<') {
        chars.next();
        tokens.push(Spanned {
            token: Token::Op("<<<"),
            start: at,
            end: chars.position,
            body: None,
        });
        return Ok(());
    }
    let (delimiter, quoted, strip_tabs) = header(chars)?;
    tokens.push(Spanned {
        token: Token::Op("<<"),
        start: at,
        end: chars.position,
        body: None,
    });
    documents.push(Document {
        delimiter,
        quoted,
        strip_tabs,
        token: tokens.len(),
    });
    tokens.push(Spanned {
        token: Token::Word(Word(Vec::new())),
        start: chars.position,
        end: chars.position,
        body: None,
    });
    Ok(())
}

/// What follows `<<` or `<<-`: the delimiter, whether any of it is quoted, and
/// whether the body's leading tabs are stripped.
pub(super) fn header(chars: &mut Cursor<'_>) -> Result<(String, bool, bool), String> {
    let strip_tabs = chars.peek() == Some('-');
    if strip_tabs {
        chars.next();
    }
    while matches!(chars.peek(), Some(' ' | '\t')) {
        chars.next();
    }
    let mut delimiter = String::new();
    let mut quoted = false;
    while let Some(ch) = chars.peek() {
        if ch.is_whitespace() || ";&|<>()".contains(ch) {
            break;
        }
        chars.next();
        match ch {
            '\'' | '"' => {
                quoted = true;
                loop {
                    match chars.next() {
                        Some(close) if close == ch => break,
                        Some(inner) => delimiter.push(inner),
                        None => return Err("unterminated quote".into()),
                    }
                }
            }
            '\\' => {
                quoted = true;
                delimiter.extend(chars.next());
            }
            _ => delimiter.push(ch),
        }
    }
    if delimiter.is_empty() {
        return Err("here-document needs a delimiter word after <<".into());
    }
    Ok((delimiter, quoted, strip_tabs))
}

/// After a newline inside `$(...)`, skips the pending bodies as plain text. As
/// in Bash, the substitution may close directly after a delimiter: `EOF)`.
pub(super) fn skip_bodies(
    chars: &mut Cursor<'_>,
    documents: &mut Vec<(String, bool)>,
) -> Result<(), String> {
    for (delimiter, strip_tabs) in documents.drain(..) {
        loop {
            let rest = chars.rest();
            let Some(line) = rest.split_inclusive('\n').next() else {
                return Err(format!(
                    "here-document is missing its closing {delimiter} line"
                ));
            };
            let content = if strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            let indent = line.len() - content.len();
            if let Some(after) = content.strip_prefix(&delimiter)
                && after.starts_with(')')
            {
                chars.advance(indent + delimiter.len());
                return Ok(());
            }
            chars.advance(line.len());
            if content.trim_end_matches('\n').trim_end_matches('\r') == delimiter {
                break;
            }
        }
    }
    Ok(())
}

/// After a newline, reads the pending bodies in the order they were opened.
pub(super) fn bodies(
    chars: &mut Cursor<'_>,
    tokens: &mut [Spanned],
    documents: &mut Vec<Document>,
) -> Result<(), String> {
    for document in documents.drain(..) {
        let body_start = chars.position;
        let mut text = String::new();
        loop {
            let mut line = String::new();
            let mut ended = false;
            for ch in chars.by_ref() {
                if ch == '\n' {
                    ended = true;
                    break;
                }
                line.push(ch);
            }
            let content = if document.strip_tabs {
                line.trim_start_matches('\t')
            } else {
                &line
            };
            if content.trim_end_matches('\r') == document.delimiter {
                break;
            }
            if !ended {
                return Err(format!(
                    "here-document is missing its closing {} line",
                    document.delimiter
                ));
            }
            text.push_str(content);
            text.push('\n');
        }
        let parts = if document.quoted {
            vec![Part::Literal(text)]
        } else {
            expanded(&text)?
        };
        tokens[document.token].token = Token::Word(Word(parts));
        tokens[document.token].body = Some((body_start, chars.position));
    }
    Ok(())
}

/// An unquoted body: `$` expansions apply and `\` escapes `$`, `` ` ``, `\`
/// and newline; quotes are ordinary characters.
fn expanded(text: &str) -> Result<Vec<Part>, String> {
    let mut chars = Cursor::new(text);
    let mut parts = Vec::new();
    let mut literal = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\\'
            && let Some(next @ ('$' | '`' | '\\' | '\n')) = chars.peek()
        {
            chars.next();
            if next != '\n' {
                literal.push(next);
            }
            continue;
        }
        if ch == '$'
            && let Some(next) = chars.peek()
            && let Some(part) = variable(next, &mut chars)?
        {
            if !literal.is_empty() {
                parts.push(Part::Literal(std::mem::take(&mut literal)));
            }
            parts.push(part);
            continue;
        }
        if ch == '`' {
            return Err(
                "unsupported shell syntax: ` in a here-document; use $(...) or quote the delimiter, as in <<'EOF'"
                    .into(),
            );
        }
        literal.push(ch);
    }
    if !literal.is_empty() {
        parts.push(Part::Literal(literal));
    }
    Ok(parts)
}
