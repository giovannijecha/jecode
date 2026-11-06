use super::Key;
use std::time::{Duration, Instant};

// Translate terminal bytes into the editor's platform-independent key model.
// Bracketed paste is interpreted by the shared decoder, never as shortcuts.
#[derive(Default)]
pub(super) struct Parser {
    sequence: Vec<u8>,
    escaped_at: Option<Instant>,
    utf8: Vec<u8>,
    modifiers: u8,
    scroll: Option<i16>,
}

impl Parser {
    pub(super) fn take_scroll(&mut self) -> Option<i16> {
        self.scroll.take()
    }
    pub(super) fn push(&mut self, byte: u8, paste: bool) -> Vec<Key> {
        if !self.utf8.is_empty() && byte.is_ascii() {
            self.utf8.clear();
            let mut keys = units("\u{fffd}", self.modifiers);
            keys.extend(self.push(byte, paste));
            return keys;
        }
        if paste {
            return self.character(byte, 0);
        }
        if self.sequence.len() > 1 && byte.is_ascii_control() {
            // A truncated escape sequence must not swallow stop/quit or editing keys.
            self.sequence.clear();
            self.escaped_at = None;
            return self.push(byte, false);
        }
        if self.sequence.is_empty() {
            if byte == 27 {
                self.sequence.push(byte);
                self.escaped_at = Some(Instant::now());
                return vec![];
            }
            return self.plain(byte, 0);
        }
        if self.sequence.len() == 1 {
            if matches!(byte, b'[' | b'O') {
                self.sequence.push(byte);
                return vec![];
            }
            self.sequence.clear();
            self.escaped_at = None;
            return if byte == 27 {
                vec![key(27, 0)]
            } else {
                self.plain(byte, 1)
            };
        }
        self.sequence.push(byte);
        if self.sequence.len() > 64 || !(0x20..=0x7e).contains(&byte) {
            self.sequence.clear();
            self.escaped_at = None;
            return vec![];
        }
        if byte >= 0x40 {
            self.escaped_at = None;
            let sequence = std::mem::take(&mut self.sequence);
            if sequence == b"\x1b[200~" {
                return units("\x1b[200~", 0);
            }
            if sequence.starts_with(b"\x1b[<") {
                self.scroll = mouse_wheel(&sequence);
                return vec![];
            }
            return sequence_key(&sequence).into_iter().collect();
        }
        vec![]
    }

    fn plain(&mut self, byte: u8, modifiers: u8) -> Vec<Key> {
        if !byte.is_ascii() {
            return self.character(byte, modifiers);
        }
        let (code, control) = match byte {
            8 | 127 => (8, false),
            9 => (9, false),
            13 => (13, false),
            1..=26 => (u16::from(byte) + 64, true),
            29 => (221, true), // Ctrl+] leaves direct terminal input.
            0 => (32, true),
            28 => (220, true),
            30 => (54, true),
            31 => (189, true),
            _ => (key_code(byte), false),
        };
        vec![Key {
            code,
            modifiers: modifiers | if control { 4 } else { 0 },
            character: if byte == 127 { 0 } else { byte.into() },
        }]
    }

    fn character(&mut self, byte: u8, modifiers: u8) -> Vec<Key> {
        if self.utf8.is_empty() {
            self.modifiers = modifiers;
        }
        self.utf8.push(byte);
        let mut keys = vec![];
        loop {
            match std::str::from_utf8(&self.utf8) {
                Ok(text) => {
                    keys.extend(units(text, self.modifiers));
                    self.utf8.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if valid > 0 {
                        keys.extend(units(
                            std::str::from_utf8(&self.utf8[..valid]).unwrap(),
                            self.modifiers,
                        ));
                    }
                    let Some(invalid) = error.error_len() else {
                        self.utf8.drain(..valid);
                        break;
                    };
                    keys.extend(units("\u{fffd}", self.modifiers));
                    self.utf8.drain(..valid + invalid);
                    if self.utf8.is_empty() {
                        break;
                    }
                }
            }
        }
        keys
    }

    pub(super) fn idle(&mut self) -> Option<Key> {
        if self.wait().is_some_and(|wait| wait.is_zero()) {
            let single = self.sequence == [27];
            self.sequence.clear();
            self.escaped_at = None;
            return single.then(|| key(27, 0));
        }
        None
    }

    pub(super) fn wait(&self) -> Option<Duration> {
        super::escape_wait(self.escaped_at)
    }
}

fn mouse_wheel(sequence: &[u8]) -> Option<i16> {
    let body = sequence.strip_prefix(b"\x1b[<")?.strip_suffix(b"M")?;
    let body = std::str::from_utf8(body).ok()?;
    let fields: Vec<u16> = body
        .split(';')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [button, x, y] = fields.as_slice() else {
        return None;
    };
    if *x == 0 || *y == 0 {
        return None;
    }
    match button & !28 {
        64 => Some(-3),
        65 => Some(3),
        _ => None,
    }
}

fn units(text: &str, modifiers: u8) -> Vec<Key> {
    text.encode_utf16()
        .map(|character| Key {
            code: 0,
            modifiers,
            character,
        })
        .collect()
}

fn key(code: u16, modifiers: u8) -> Key {
    Key {
        code,
        modifiers,
        character: 0,
    }
}

fn modifiers(value: u16) -> Option<u8> {
    // The keyboard protocol adds 64 for Caps Lock and 128 for Num Lock, which do
    // not change a shortcut. Super, Hyper and Meta are not Jecode's to interpret.
    let bits = value.checked_sub(1)? & !(64 | 128);
    if bits > 7 {
        return None;
    }
    let bits = bits as u8;
    Some((bits & 1) << 1 | (bits & 2) >> 1 | bits & 4)
}

/// The shared input model uses Windows virtual-key codes, where the values of ASCII
/// punctuation mean navigation: `.` would read as Delete and `'` as Right. Letters,
/// digits and Space have the same value in both.
fn key_code(byte: u8) -> u16 {
    match byte {
        b')' => 48,
        b'!' => 49,
        b'@' => 50,
        b'#' => 51,
        b'$' => 52,
        b'%' => 53,
        b'^' => 54,
        b'&' => 55,
        b'*' => 56,
        b'(' => 57,
        b';' | b':' => 186,
        b'=' | b'+' => 187,
        b',' | b'<' => 188,
        b'-' | b'_' => 189,
        b'.' | b'>' => 190,
        b'/' | b'?' => 191,
        b'`' | b'~' => 192,
        b'[' | b'{' => 219,
        b'\\' | b'|' => 220,
        b']' | b'}' => 221,
        b'\'' | b'"' => 222,
        _ => byte.to_ascii_uppercase().into(),
    }
}

fn sequence_key(sequence: &[u8]) -> Option<Key> {
    let (&last, body) = sequence[2..].split_last()?;
    let body = std::str::from_utf8(body).ok()?;
    // A parameter can carry `:`-separated details. For a key the third one is its
    // position on the base layout: `1089::99` is the key that types `c` on a US keyboard.
    let base = body
        .split(';')
        .next()?
        .split(':')
        .nth(2)
        .and_then(|value| value.parse().ok());
    let parameters: Vec<u16> = if body.is_empty() {
        vec![]
    } else {
        body.split(';')
            .map(|field| field.split(':').next().unwrap_or(field).parse())
            .collect::<Result<_, _>>()
            .ok()?
    };
    let modifier = modifiers(parameters.get(1).copied().unwrap_or(1))?;
    let code = match last {
        b'A' => 38,
        b'B' => 40,
        b'C' => 39,
        b'D' => 37,
        b'H' => 36,
        b'F' => 35,
        b'P' => 112,
        b'Q' => 113,
        b'R' if sequence[1] == b'O' => 114,
        b'S' => 115,
        b'Z' if parameters.is_empty() => return Some(key(9, 2)),
        b'~' if parameters.len() == 3 && parameters[0] == 27 => {
            return encoded_key(parameters[2], modifier);
        }
        b'~' if parameters.len() <= 2 => match *parameters.first()? {
            1 | 7 => 36,
            4 | 8 => 35,
            2 => 45,
            3 => 46,
            5 => 33,
            6 => 34,
            11..=15 => 112 + parameters[0] - 11,
            _ => return None,
        },
        b'u' if parameters.len() <= 2 => {
            let reported = *parameters.first()?;
            // Shortcuts follow the base layout on a Cyrillic or Greek keyboard.
            let named = match (reported, base) {
                (128..=57343, Some(base)) => base,
                _ => reported,
            };
            return encoded_key(named, modifier);
        }
        _ => return None,
    };
    if last != b'~' && parameters.len() > 2 {
        return None;
    }
    Some(key(code, modifier))
}

/// The keypad's navigation keys with Num Lock off, from 57417 in the protocol's order.
const KEYPAD_NAVIGATION: [u16; 10] = [37, 39, 38, 40, 33, 34, 36, 35, 45, 46];

fn encoded_key(character: u16, modifiers: u8) -> Option<Key> {
    let code = match character {
        9 | 13 => character,
        // The keypad's Enter, which the protocol reports separately.
        57414 => {
            return Some(Key {
                code: 13,
                modifiers,
                character: 13,
            });
        }
        57417..=57426 => {
            return Some(key(
                KEYPAD_NAVIGATION[usize::from(character - 57417)],
                modifiers,
            ));
        }
        // Ctrl+[ is the Escape key in every legacy encoding.
        91 if modifiers == 4 => return Some(key(27, 0)),
        27 => 27,
        127 => 8,
        32..=126 => key_code(character as u8),
        _ => return None,
    };
    // The protocol reports AltGr text as plain text, so Ctrl+Alt here is a chord. The
    // shared decoder would read a character with both modifiers as Windows AltGr text.
    let chord = modifiers & 5 == 5;
    Some(Key {
        code,
        modifiers,
        character: if character == 127 || character == 27 || chord {
            0
        } else {
            character
        },
    })
}

#[cfg(test)]
mod tests;
