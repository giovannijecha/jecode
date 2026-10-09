use std::io::BufRead;

const INPUT_LIMIT: usize = 1024 * 1024;

pub fn read_line(input: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();
    let mut over_limit = false;
    loop {
        let buffer = input
            .fill_buf()
            .map_err(|error| format!("Could not read input: {error}"))?;
        if buffer.is_empty() {
            if bytes.is_empty() && !over_limit {
                return Ok(None);
            }
            break;
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let length = newline.map_or(buffer.len(), |position| position + 1);
        if !over_limit {
            if bytes.len() + length > INPUT_LIMIT {
                over_limit = true;
            } else {
                bytes.extend_from_slice(&buffer[..length]);
            }
        }
        input.consume(length);
        if newline.is_some() {
            break;
        }
    }
    if over_limit {
        return Err("Input exceeds the 1 MiB limit".into());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "Input must be valid UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn bounds_input_and_handles_eof_unicode_and_crlf() {
        let mut input = Cursor::new("café\r\nlast".as_bytes());
        assert_eq!(read_line(&mut input).unwrap().as_deref(), Some("café\r\n"));
        assert_eq!(read_line(&mut input).unwrap().as_deref(), Some("last"));
        assert!(read_line(&mut input).unwrap().is_none());
        let mut oversized = Cursor::new(format!("{}\nnext\n", "x".repeat(INPUT_LIMIT)));
        assert!(read_line(&mut oversized).is_err());
        assert_eq!(
            read_line(&mut oversized).unwrap().as_deref(),
            Some("next\n")
        );
        assert!(read_line(&mut Cursor::new(vec![255, b'\n'])).is_err());
    }
}
