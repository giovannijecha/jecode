use super::Error;

pub(super) fn read(text: &str, at: &mut usize) -> Result<String, Error> {
    let bytes = text.as_bytes();
    if bytes.get(*at) != Some(&b'"') {
        return Err(Error::Syntax);
    }
    *at += 1;
    let mut result = String::new();
    let mut start = *at;
    while let Some(&byte) = bytes.get(*at) {
        match byte {
            b'"' => {
                result.push_str(&text[start..*at]);
                *at += 1;
                return Ok(result);
            }
            b'\\' => {
                result.push_str(&text[start..*at]);
                *at += 1;
                let escaped = *bytes.get(*at).ok_or(Error::Syntax)?;
                *at += 1;
                result.push(match escaped {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'b' => '\x08',
                    b'f' => '\x0c',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'u' => unicode(bytes, at)?,
                    _ => return Err(Error::Syntax),
                });
                start = *at;
            }
            0..=31 => return Err(Error::Syntax),
            _ => *at += 1,
        }
    }
    Err(Error::Syntax)
}

fn hex(bytes: &[u8], at: &mut usize) -> Result<u32, Error> {
    let mut value = 0;
    for _ in 0..4 {
        let digit = *bytes.get(*at).ok_or(Error::Unicode)?;
        *at += 1;
        let digit = match digit {
            b'0'..=b'9' => digit - b'0',
            b'a'..=b'f' => digit - b'a' + 10,
            b'A'..=b'F' => digit - b'A' + 10,
            _ => return Err(Error::Unicode),
        };
        value = value * 16 + u32::from(digit);
    }
    Ok(value)
}

fn unicode(bytes: &[u8], at: &mut usize) -> Result<char, Error> {
    let first = hex(bytes, at)?;
    let code = if (0xd800..=0xdbff).contains(&first) {
        if bytes.get(*at..*at + 2) != Some(b"\\u") {
            return Err(Error::Unicode);
        }
        *at += 2;
        let second = hex(bytes, at)?;
        if !(0xdc00..=0xdfff).contains(&second) {
            return Err(Error::Unicode);
        }
        0x10000 + ((first - 0xd800) << 10) + second - 0xdc00
    } else {
        first
    };
    char::from_u32(code).ok_or(Error::Unicode)
}
