//! Incremental UTF-8 decoding, visible control escaping and bounded result tails.
#[derive(Default)]
pub(super) struct Capture {
    pending: Vec<u8>,
    cr: bool,
    pub tail: String,
    pub truncated: bool,
}
pub(super) fn invisible(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' |
        '\u{2060}'..='\u{206f}' | '\u{feff}')
}
impl Capture {
    pub fn push(&mut self, bytes: &[u8], eof: bool) -> String {
        self.pending.extend_from_slice(bytes);
        let mut decoded = String::new();
        let mut offset = 0;
        while offset < self.pending.len() {
            match std::str::from_utf8(&self.pending[offset..]) {
                Ok(text) => {
                    decoded.push_str(text);
                    offset = self.pending.len();
                }
                Err(error) => {
                    let end = offset + error.valid_up_to();
                    decoded.push_str(std::str::from_utf8(&self.pending[offset..end]).unwrap());
                    offset = end;
                    if let Some(size) = error.error_len() {
                        decoded.push('\u{fffd}');
                        offset += size;
                    } else if eof {
                        decoded.push('\u{fffd}');
                        offset = self.pending.len();
                    } else {
                        break;
                    }
                }
            }
        }
        self.pending.drain(..offset);
        let mut safe = String::new();
        for c in decoded.chars() {
            if c == '\n' && self.cr {
                self.cr = false;
                continue;
            }
            self.cr = c == '\r';
            match c {
                '\r' | '\n' => safe.push('\n'),
                '\t' => safe.push_str("    "),
                c if c.is_control() || invisible(c) => {
                    safe.push_str(&format!("\\u{{{:x}}}", c as u32))
                }
                c => safe.push(c),
            }
        }
        self.tail.push_str(&safe);
        if self.tail.len() > 6144 {
            let mut start = self.tail.len() - 6144;
            while !self.tail.is_char_boundary(start) {
                start += 1;
            }
            self.tail.drain(..start);
            self.truncated = true;
        }
        safe
    }
}
