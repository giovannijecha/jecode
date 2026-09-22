//! Bounded VT input decoding. Bracketed paste never becomes commands/shortcuts.
use super::Key;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Decoder {
    pending: Vec<u8>,
    paste: Option<Vec<u8>>,
    escape_since: Option<Instant>,
}
impl Decoder {
    #[cfg(windows)]
    pub fn native(&mut self, key: Key, now: Instant) -> Vec<Key> {
        match key {
            Key::Text(text) => self.push(text.as_bytes(), now),
            Key::Enter => self.push(b"\r", now),
            Key::Escape => self.push(b"\x1b", now),
            Key::Interrupt => self.push(&[3], now),
            Key::Quit => self.push(&[17], now),
            key if self.paste.is_none() => vec![key],
            _ => Vec::new(),
        }
    }
    pub fn push(&mut self, bytes: &[u8], now: Instant) -> Vec<Key> {
        let mut result = Vec::new();
        for byte in bytes {
            self.pending.push(*byte);
            self.decode(now, &mut result);
        }
        result
    }
    pub fn idle(&mut self, now: Instant) -> Vec<Key> {
        if self.pending == b"\x1b"
            && self
                .escape_since
                .is_some_and(|at| now.duration_since(at) >= Duration::from_millis(60))
        {
            self.pending.clear();
            self.escape_since = None;
            return vec![Key::Escape];
        }
        Vec::new()
    }
    fn decode(&mut self, now: Instant, output: &mut Vec<Key>) {
        if let Some(paste) = self.paste.as_mut() {
            const END: &[u8] = b"\x1b[201~";
            while !self.pending.is_empty() && !END.starts_with(&self.pending) {
                if paste.len() < 8192 {
                    paste.push(self.pending[0]);
                }
                self.pending.remove(0);
            }
            if self.pending == END {
                let bytes = self.paste.take().unwrap();
                // A truncated paste may end mid-scalar: retain only its valid prefix.
                let end = std::str::from_utf8(&bytes)
                    .err()
                    .map_or(bytes.len(), |e| e.valid_up_to());
                if end != 0 {
                    output.push(Key::Text(String::from_utf8(bytes[..end].to_vec()).unwrap()));
                }
                self.pending.clear();
            }
            return;
        }
        if self.pending[0] == 27 {
            if self.pending.len() == 1 {
                self.escape_since = Some(now);
                return;
            }
            if self.pending[1] == b'[' || self.pending[1] == b'O' {
                if self.pending.len() < 3 {
                    return;
                }
                if self.pending.len() > 32 {
                    self.pending.clear();
                    return;
                }
                let last = *self.pending.last().unwrap();
                if !(0x40..=0x7e).contains(&last) {
                    return;
                }
                let key = match self.pending.as_slice() {
                    b"\x1b[D" | b"\x1bOD" => Some(Key::Left),
                    b"\x1b[C" | b"\x1bOC" => Some(Key::Right),
                    b"\x1b[H" | b"\x1bOH" | b"\x1b[1~" => Some(Key::Home),
                    b"\x1b[F" | b"\x1bOF" | b"\x1b[4~" => Some(Key::End),
                    b"\x1b[3~" => Some(Key::Delete),
                    b"\x1b[5~" => Some(Key::PageUp),
                    b"\x1b[6~" => Some(Key::PageDown),
                    b"\x1b[200~" => {
                        self.paste = Some(Vec::new());
                        None
                    }
                    _ => None,
                };
                if let Some(key) = key {
                    output.push(key);
                }
            }
            self.pending.clear();
            self.escape_since = None;
            return;
        }
        let key = match self.pending[0] {
            3 => Some(Key::Interrupt),
            17 => Some(Key::Quit),
            1 => Some(Key::Home),
            5 => Some(Key::End),
            9 => Some(Key::Tab),
            8 | 127 => Some(Key::Backspace),
            b'\r' | b'\n' => Some(Key::Enter),
            0..=31 => None,
            _ => match std::str::from_utf8(&self.pending) {
                Ok(text) => Some(Key::Text(text.into())),
                Err(error) if error.error_len().is_none() && self.pending.len() < 4 => return,
                Err(_) => None,
            },
        };
        if let Some(key) = key {
            output.push(key);
        }
        self.pending.clear();
    }
}
