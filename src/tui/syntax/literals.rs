// Literal shapes the display lexer recognises across profiles.
use super::ascii_word;

/// Only indentation and `- ` list markers precede a YAML key.
pub(super) fn list_lead(before: &str) -> bool {
    let mut rest = before.trim_start();
    while let Some(after) = rest.strip_prefix('-') {
        let spaced = after.trim_start();
        if spaced.len() == after.len() {
            return false;
        }
        rest = spaced;
    }
    rest.is_empty()
}

/// The shortest key followed by `:` and whitespace or the end of the line.
pub(super) fn yaml_key(tail: &str) -> Option<usize> {
    let key = |character: char| ascii_word(character) || matches!(character, '.' | '/' | '-');
    let first = tail.chars().next()?;
    if !key(first) || first == '-' && tail[1..].starts_with(char::is_whitespace) {
        return None;
    }
    let mut end = first.len_utf8();
    loop {
        if let Some(after) = tail[end..].trim_start().strip_prefix(':')
            && after.chars().next().is_none_or(char::is_whitespace)
        {
            return Some(end);
        }
        let next = tail[end..].chars().next()?;
        if !key(next) && next != ' ' {
            return None;
        }
        end += next.len_utf8();
    }
}

/// Hexadecimal, binary or decimal with a fraction and exponent, then a unit
/// such as `px`, `vw` or `u32`, or a percent sign.
pub(super) fn number(value: &str) -> usize {
    let bytes = value.as_bytes();
    let run = |from: usize, digit: fn(&u8) -> bool| {
        from + bytes[from..]
            .iter()
            .take_while(|byte| digit(byte) || **byte == b'_')
            .count()
    };
    let prefixed = |marks: &[u8], digit: fn(&u8) -> bool| {
        (bytes.len() > 2
            && bytes[0] == b'0'
            && marks.contains(&bytes[1])
            && (digit(&bytes[2]) || bytes[2] == b'_'))
            .then(|| run(2, digit))
    };
    let mut end = prefixed(b"xX", u8::is_ascii_hexdigit)
        .or_else(|| prefixed(b"bB", |digit| matches!(digit, b'0' | b'1')))
        .unwrap_or_else(|| {
            let mut end = run(1, u8::is_ascii_digit);
            if bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_digit) {
                end = run(end + 1, u8::is_ascii_digit);
            }
            if matches!(bytes.get(end), Some(b'e' | b'E')) {
                let sign = end + 1 + usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
                if bytes.get(sign).is_some_and(u8::is_ascii_digit) {
                    end = sign
                        + bytes[sign..]
                            .iter()
                            .take_while(|d| d.is_ascii_digit())
                            .count();
                }
            }
            end
        });
    if bytes.get(end) == Some(&b'%') {
        end += 1;
    } else if bytes.get(end).is_some_and(u8::is_ascii_alphabetic) {
        end += 1 + bytes[end + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
            .count();
    }
    end
}

pub(super) fn raw_prefix(value: &str) -> Option<(usize, usize)> {
    let start = if value.starts_with("br") {
        2
    } else if value.starts_with('r') {
        1
    } else {
        return None;
    };
    let hashes = value[start..]
        .bytes()
        .take_while(|&byte| byte == b'#')
        .count();
    (value.as_bytes().get(start + hashes) == Some(&b'"')).then_some((start + hashes + 1, hashes))
}

pub(super) fn raw_end(value: &str, hashes: usize) -> (usize, bool) {
    for (index, _) in value.match_indices('"') {
        let end = index + 1 + hashes;
        if value
            .as_bytes()
            .get(index + 1..end)
            .is_some_and(|tail| tail.iter().all(|&byte| byte == b'#'))
        {
            return (end, true);
        }
    }
    (value.len(), false)
}

pub(super) fn char_literal(value: &str) -> Option<usize> {
    let tail = value.strip_prefix('\'')?;
    let first = tail.chars().next()?;
    let mut length = 1 + first.len_utf8();
    if first == '\\' {
        let escape = value[length..].chars().next()?;
        length += escape.len_utf8();
        if escape == 'u' && value[length..].starts_with('{') {
            length += value[length..].find('}')? + 1;
        } else if escape == 'x' {
            if !value
                .as_bytes()
                .get(length..length + 2)?
                .iter()
                .all(u8::is_ascii_hexdigit)
            {
                return None;
            }
            length += 2;
        }
    } else if first == '\'' {
        return None;
    }
    value[length..].starts_with('\'').then_some(length + 1)
}
