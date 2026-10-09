use super::terminal::Key;
use std::time::{Duration, Instant};
#[cfg(any(unix, test))]
mod vt;

pub enum Decoded {
    Key(Key),
    Text(String),
    /// One complete paste, which may be a file drop.
    Paste(String),
    Error,
    Scroll(i16),
}

#[derive(Default)]
pub struct Decoder {
    prefix: String,
    paste: Option<String>,
    surrogate: Option<u16>,
    escaped_at: Option<Instant>,
    #[cfg(any(windows, test))]
    paste_cr: bool,
    overflow: bool,
    #[cfg(any(unix, test))]
    vt: vt::Parser,
}
impl Decoder {
    #[cfg(any(windows, test))]
    pub fn paste(&mut self, units: &[u16]) -> Result<String, ()> {
        let mut text = String::new();
        for &unit in units {
            if self.paste_cr && unit == 10 {
                self.paste_cr = false;
                continue;
            }
            self.paste_cr = unit == 13;
            for decoded in self.key(Key {
                code: 0,
                modifiers: 0,
                character: unit,
            }) {
                match decoded {
                    Decoded::Error => return Err(()),
                    Decoded::Text(value) | Decoded::Paste(value) => text.push_str(&value),
                    Decoded::Key(key) if matches!(key.character, 9 | 10 | 13) => {
                        text.push(char::from_u32(key.character.into()).unwrap())
                    }
                    _ => {}
                }
            }
        }
        Ok(text)
    }
    #[cfg(any(unix, test))]
    pub fn bytes(&mut self, bytes: &[u8]) -> Vec<Decoded> {
        let mut output = vec![];
        for &byte in bytes {
            for key in self.vt.push(byte, self.paste.is_some()) {
                output.extend(self.key(key));
            }
            if let Some(amount) = self.vt.take_scroll() {
                output.push(Decoded::Scroll(amount));
            }
        }
        output
    }
    pub fn key(&mut self, key: Key) -> Vec<Decoded> {
        if key.character == 27 || !self.prefix.is_empty() {
            let character = char::from_u32(key.character.into()).unwrap_or('\u{fffd}');
            self.prefix.push(character);
            self.escaped_at = Some(Instant::now());
            let marker = if self.paste.is_some() {
                "\x1b[201~"
            } else {
                "\x1b[200~"
            };
            if self.prefix == marker {
                self.prefix.clear();
                self.escaped_at = None;
                return if let Some(text) = self.paste.take() {
                    if std::mem::take(&mut self.overflow) {
                        vec![Decoded::Error]
                    } else {
                        vec![Decoded::Paste(text)]
                    }
                } else {
                    self.overflow = false;
                    self.paste = Some(String::new());
                    vec![]
                };
            }
            if marker.starts_with(&self.prefix) {
                return vec![];
            }
            let prefix = std::mem::take(&mut self.prefix);
            if let Some(paste) = &mut self.paste {
                if paste.len() + prefix.len() <= 1024 * 1024 {
                    paste.push_str(&prefix);
                } else {
                    self.overflow = true;
                }
                return vec![];
            }
            // An ordinary Escape is a focus command; unrelated following input is preserved.
            let mut output = vec![Decoded::Key(Key {
                code: 27,
                modifiers: 0,
                character: 27,
            })];
            output.extend(self.key(key));
            return output;
        }
        if self.paste.is_some()
            || (key.character >= 32
                && (!key.ctrl() && key.modifiers & 1 == 0 || key.ctrl() && key.modifiers & 1 != 0))
        {
            let unit = key.character;
            if (0xd800..=0xdbff).contains(&unit) {
                self.surrogate = Some(unit);
                return vec![];
            }
            let text = if let Some(high) = self.surrogate.take() {
                char::decode_utf16([high, unit])
                    .map(|character| character.unwrap_or('\u{fffd}'))
                    .collect::<String>()
            } else {
                char::from_u32(unit.into())
                    .unwrap_or('\u{fffd}')
                    .to_string()
            };
            if let Some(paste) = &mut self.paste {
                if paste.len() + text.len() <= 1024 * 1024 {
                    paste.push_str(&text);
                } else {
                    self.overflow = true;
                }
                vec![]
            } else {
                vec![Decoded::Text(text)]
            }
        } else {
            vec![Decoded::Key(key)]
        }
    }
    pub fn idle(&mut self) -> Option<Decoded> {
        #[cfg(any(unix, test))]
        if let Some(key) = self.vt.idle() {
            return Some(Decoded::Key(key));
        }
        if self.paste.is_none()
            && self
                .escaped_at
                .is_some_and(|at| at.elapsed() >= Duration::from_millis(150))
            && !self.prefix.is_empty()
        {
            self.prefix.clear();
            self.escaped_at = None;
            Some(Decoded::Key(Key {
                code: 27,
                modifiers: 0,
                character: 27,
            }))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bracketed_paste_is_one_insert_and_never_an_enter_command() {
        let mut decoder = Decoder::default();
        let mut output = Vec::new();
        for unit in "\x1b[200~first\r\nsecond\x1b[201~".encode_utf16() {
            output.extend(decoder.key(Key {
                code: if unit == 13 { 13 } else { 0 },
                modifiers: 0,
                character: unit,
            }));
        }
        assert_eq!(output.len(), 1);
        assert!(matches!(&output[0], Decoded::Paste(text) if text == "first\r\nsecond"));
    }
    #[test]
    fn combines_console_utf16_surrogates() {
        let mut decoder = Decoder::default();
        let output: Vec<_> = "🙂"
            .encode_utf16()
            .flat_map(|unit| {
                decoder.key(Key {
                    code: 0,
                    modifiers: 0,
                    character: unit,
                })
            })
            .collect();
        assert!(matches!(&output[0], Decoded::Text(text) if text == "🙂"));
    }

    #[test]
    fn native_batches_preserve_a_complete_bracketed_paste() {
        let mut decoder = Decoder::default();
        let text = "'C:\\Temp\\a$x ` β.bin'";
        let bracketed = format!("\x1b[200~{text}\x1b[201~");
        assert_eq!(
            decoder.paste(&bracketed.encode_utf16().collect::<Vec<_>>()),
            Ok(text.to_owned())
        );
        let mut decoder = Decoder::default();
        let first = decoder
            .paste(&"\x1b[200~first\r".encode_utf16().collect::<Vec<_>>())
            .unwrap();
        let second = decoder
            .paste(&"\nsecond\x1b[201~".encode_utf16().collect::<Vec<_>>())
            .unwrap();
        assert_eq!(first + &second, "first\rsecond");
    }

    #[test]
    fn native_paste_preserves_tabs_and_crlf_across_record_batches() {
        let mut decoder = Decoder::default();
        let first = decoder
            .paste(&"first\r".encode_utf16().collect::<Vec<_>>())
            .unwrap();
        let second = decoder
            .paste(&"\n\tsecond".encode_utf16().collect::<Vec<_>>())
            .unwrap();
        assert_eq!(first + &second, "first\r\tsecond");
    }
}
