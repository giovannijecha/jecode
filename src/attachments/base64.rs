//! Standard base64 (RFC 4648) with padding.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(data: &[u8]) -> String {
    let mut output = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        output.push(ALPHABET[(a >> 2) as usize] as char);
        output.push(ALPHABET[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}

/// Decodes base64, ignoring ASCII whitespace such as wrapped lines.
pub fn decode(text: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0;
    let mut padding = 0;
    for byte in text.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding += 1;
                continue;
            }
            _ => return Err("Invalid base64 data".into()),
        };
        if padding > 0 {
            return Err("Invalid base64 padding".into());
        }
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    if padding > 2 || bits >= 6 {
        return Err("Truncated base64 data".into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_length_and_byte() {
        let data = (0..=255u8).collect::<Vec<_>>();
        for length in 0..data.len() {
            let encoded = encode(&data[..length]);
            assert_eq!(encoded.len() % 4, 0);
            assert_eq!(decode(&encoded).unwrap(), &data[..length]);
        }
        assert_eq!(encode(b"Man"), "TWFu");
        assert_eq!(decode("TW\r\nE=").unwrap(), b"Ma");
        assert!(decode("TW!u").is_err());
        assert!(decode("T").is_err());
        assert!(decode("TQ==TQ==").is_err());
    }
}
