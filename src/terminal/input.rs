//! Bounded VT input decoding. Bracketed paste never becomes commands/shortcuts.
use super::{Key, editor::MAX_INPUT_BYTES};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Decoder {
    pending: Vec<u8>,
    paste: Option<Paste>,
    escape_since: Option<Instant>,
}
#[derive(Default)]
struct Paste {
    bytes: Vec<u8>,
    overflow: bool,
}
impl Decoder {
    #[cfg(windows)]
    pub fn native(&mut self, key: Key, now: Instant) -> Vec<Key> {
        match key {
            Key::Text(text) => self.push(text.as_bytes(), now),
            Key::Enter => self.push(b"\r", now),
            Key::Newline if self.paste.is_some() => self.push(b"\n", now),
            Key::Escape => self.push(b"\x1b", now),
            Key::Interrupt => self.push(&[3], now),
            Key::Quit => self.push(&[17], now),
            Key::Tab if self.paste.is_some() => self.push(b"\t", now),
            Key::Backspace if self.paste.is_some() => self.push(&[8], now),
            Key::WordBackspace if self.paste.is_some() => self.push(&[8], now),
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
                if paste.bytes.len() < MAX_INPUT_BYTES {
                    paste.bytes.push(self.pending[0]);
                } else {
                    paste.overflow = true;
                }
                self.pending.remove(0);
            }
            if self.pending == END {
                let paste = self.paste.take().unwrap();
                output.push(if paste.overflow {
                    Key::PasteRejected("Paste exceeds 8 KiB / draft kept")
                } else if let Ok(text) = String::from_utf8(paste.bytes) {
                    Key::Paste(text)
                } else {
                    Key::PasteRejected("Invalid paste text / draft kept")
                });
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
                    b"\x1b[A" | b"\x1bOA" => Some(Key::Up),
                    b"\x1b[B" | b"\x1bOB" => Some(Key::Down),
                    b"\x1b[1;3A" | b"\x1b[3A" => Some(Key::RetrieveQueued),
                    b"\x1b[1;3B" | b"\x1b[3B" => Some(Key::AbandonRecovered),
                    b"\x1b[D" | b"\x1bOD" => Some(Key::Left),
                    b"\x1b[C" | b"\x1bOC" => Some(Key::Right),
                    b"\x1b[H" | b"\x1bOH" | b"\x1b[1~" => Some(Key::Home),
                    b"\x1b[F" | b"\x1bOF" | b"\x1b[4~" => Some(Key::End),
                    b"\x1b[3~" => Some(Key::Delete),
                    b"\x1b[1;5D" | b"\x1b[5D" => Some(Key::WordLeft),
                    b"\x1b[1;5C" | b"\x1b[5C" => Some(Key::WordRight),
                    b"\x1b[3;5~" => Some(Key::WordDelete),
                    b"\x1b[127;5u" | b"\x1b[8;5u" => Some(Key::WordBackspace),
                    b"\x1b[1;5H" | b"\x1b[1;5~" => Some(Key::DraftStart),
                    b"\x1b[1;5F" | b"\x1b[4;5~" => Some(Key::DraftEnd),
                    b"\x1b[13;2u" | b"\x1b[13;5u" | b"\x1b[27;2;13~" => Some(Key::Newline),
                    b"\x1b[5~" => Some(Key::PageUp),
                    b"\x1b[6~" => Some(Key::PageDown),
                    b"\x1b[200~" => {
                        self.paste = Some(Paste::default());
                        None
                    }
                    _ => None,
                };
                if let Some(key) = key {
                    output.push(key);
                }
            } else {
                let key = match self.pending.as_slice() {
                    b"\x1bb" => Some(Key::WordLeft),
                    b"\x1bf" => Some(Key::WordRight),
                    b"\x1b\x7f" => Some(Key::WordBackspace),
                    b"\x1b\r" | b"\x1b\n" => Some(Key::Newline),
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
            10 | 15 => Some(Key::Newline),
            16 => Some(Key::HistoryPrevious),
            14 => Some(Key::HistoryNext),
            23 => Some(Key::WordBackspace),
            9 => Some(Key::Tab),
            8 | 127 => Some(Key::Backspace),
            b'\r' => Some(Key::Enter),
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
