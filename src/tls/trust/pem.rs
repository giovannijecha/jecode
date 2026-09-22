//! Strict bounded certificate PEM/base64 decoding, independent of JSON/JWT.
pub(super) fn decode(input: &[u8]) -> Option<Vec<Vec<u8>>> {
    if input.len() > 8 * 1024 * 1024 {
        return None;
    }
    let mut roots = Vec::new();
    let mut current = None::<String>;
    for line in std::str::from_utf8(input).ok()?.lines() {
        let line = line.trim();
        if line == "-----BEGIN CERTIFICATE-----" {
            if current.is_some() {
                return None;
            }
            current = Some(String::new());
        } else if line == "-----END CERTIFICATE-----" {
            let text = current.take()?;
            roots.push(base64(text.as_bytes())?);
            if roots.len() > 4096 {
                return None;
            }
        } else if let Some(text) = &mut current {
            if text.len() + line.len() > 90_000 {
                return None;
            }
            text.push_str(line);
        } else if !line.is_empty() && !line.starts_with('#') {
            return None;
        }
    }
    if current.is_some() || roots.is_empty() {
        None
    } else {
        Some(roots)
    }
}
fn base64(input: &[u8]) -> Option<Vec<u8>> {
    if input.is_empty() || !input.len().is_multiple_of(4) {
        return None;
    }
    let mut output = Vec::new();
    for (i, chunk) in input.chunks_exact(4).enumerate() {
        let a = digit(chunk[0])?;
        let b = digit(chunk[1])?;
        output.push((a << 2) | (b >> 4));
        if chunk[2] == b'=' {
            if chunk[3] != b'=' || b & 15 != 0 || (i + 1) * 4 != input.len() {
                return None;
            }
        } else {
            let c = digit(chunk[2])?;
            output.push((b << 4) | (c >> 2));
            if chunk[3] == b'=' {
                if c & 3 != 0 || (i + 1) * 4 != input.len() {
                    return None;
                }
            } else {
                output.push((c << 6) | digit(chunk[3])?);
            }
        }
    }
    (output.len() <= 65_536).then_some(output)
}
fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_incomplete_noncanonical_and_foreign_labels() {
        let wrap =
            |text| format!("-----BEGIN CERTIFICATE-----\n{text}\n-----END CERTIFICATE-----\n");
        for (encoded, expected) in [("YQ==", b"a".as_slice()), ("YWI=", b"ab"), ("YWJj", b"abc")] {
            assert_eq!(decode(wrap(encoded).as_bytes()).unwrap(), [expected]);
        }
        for bad in [
            "YR==", "YWJ=", "YQ=A", "YQ==AAAA", "YQ", "", "____", "YW Jj",
        ] {
            assert!(decode(wrap(bad).as_bytes()).is_none());
        }
        assert!(decode(b"-----BEGIN CERTIFICATE-----\nYWJj").is_none());
        assert!(
            decode(b"-----BEGIN TRUSTED CERTIFICATE-----\nYWJj\n-----END TRUSTED CERTIFICATE-----")
                .is_none()
        );
    }
}
