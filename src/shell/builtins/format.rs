use super::*;

pub(super) fn printf(args: &[String]) -> Outcome {
    let Some(format) = args.first() else {
        return Err("printf: missing format".into());
    };
    let mut output = Vec::new();
    let mut values = args[1..].iter();
    loop {
        let remaining = values.len();
        let mut chars = format.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                output.push(match chars.next() {
                    Some('n') => b'\n',
                    Some('t') => b'\t',
                    Some('r') => b'\r',
                    Some('\\') => b'\\',
                    Some(first @ '0'..='7') => {
                        let mut value = first.to_digit(8).unwrap();
                        for _ in 0..2 {
                            if let Some(next @ '0'..='7') = chars.peek().copied() {
                                chars.next();
                                value = value * 8 + next.to_digit(8).unwrap();
                            } else {
                                break;
                            }
                        }
                        value as u8
                    }
                    Some(other) => other as u8,
                    None => b'\\',
                });
            } else if ch == '%' {
                if chars.peek() == Some(&'%') {
                    chars.next();
                    output.push(b'%');
                    continue;
                }
                let mut width = String::new();
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    width.push(chars.next().unwrap());
                }
                let spec = chars.next().ok_or("printf: incomplete format")?;
                if !matches!(spec, 's' | 'd' | 'i') {
                    return Err(format!("printf: unsupported format %{spec}"));
                }
                let value = values.next().map(String::as_str).unwrap_or("");
                let value = if matches!(spec, 'd' | 'i') {
                    value.parse::<i64>().unwrap_or(0).to_string()
                } else {
                    value.into()
                };
                let width = if width.is_empty() {
                    0
                } else {
                    width
                        .parse::<usize>()
                        .map_err(|_| "printf: invalid width")?
                };
                if width > 64 * 1024 * 1024 {
                    return Err("printf: width exceeds 64 MiB".into());
                }
                output.resize(output.len() + width.saturating_sub(value.len()), b' ');
                output.extend(value.as_bytes());
            } else {
                output.extend(ch.to_string().as_bytes());
            }
        }
        if values.len() == remaining || values.len() == 0 {
            break;
        }
    }
    ok(output)
}
