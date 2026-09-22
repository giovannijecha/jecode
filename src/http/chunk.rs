use super::Error;

pub(super) fn size(line: &[u8]) -> Result<usize, Error> {
    let end = line
        .iter()
        .position(|b| !b.is_ascii_hexdigit())
        .unwrap_or(line.len());
    if end == 0 {
        return Err(Error::Invalid);
    }
    let mut size = 0usize;
    for &byte in &line[..end] {
        let digit = (byte as char).to_digit(16).ok_or(Error::Invalid)? as usize;
        size = size
            .checked_mul(16)
            .and_then(|s| s.checked_add(digit))
            .ok_or(Error::Limit)?;
    }
    extensions(&line[end..])?;
    Ok(size)
}

fn extensions(mut bytes: &[u8]) -> Result<(), Error> {
    while !bytes.is_empty() {
        bytes = space(bytes);
        if bytes.first() != Some(&b';') {
            return Err(Error::Invalid);
        }
        bytes = space(&bytes[1..]);
        let end = bytes
            .iter()
            .position(|b| !super::token(*b))
            .unwrap_or(bytes.len());
        if end == 0 {
            return Err(Error::Invalid);
        }
        bytes = &bytes[end..];
        let after_space = space(bytes);
        if after_space.first() == Some(&b'=') {
            bytes = space(&after_space[1..]);
            if bytes.first() == Some(&b'"') {
                bytes = quoted(&bytes[1..])?;
            } else {
                let end = bytes
                    .iter()
                    .position(|b| !super::token(*b))
                    .unwrap_or(bytes.len());
                if end == 0 {
                    return Err(Error::Invalid);
                }
                bytes = &bytes[end..];
            }
        }
    }
    Ok(())
}

fn space(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'))
        .unwrap_or(bytes.len());
    &bytes[end..]
}
fn quoted(mut bytes: &[u8]) -> Result<&[u8], Error> {
    while let Some((&byte, rest)) = bytes.split_first() {
        bytes = rest;
        match byte {
            b'"' => return Ok(bytes),
            b'\\' => {
                let (&escaped, rest) = bytes.split_first().ok_or(Error::Invalid)?;
                if escaped != b'\t' && !(32..=126).contains(&escaped) {
                    return Err(Error::Invalid);
                }
                bytes = rest;
            }
            b'\t' | 32..=126 => {}
            _ => return Err(Error::Invalid),
        }
    }
    Err(Error::Invalid)
}
