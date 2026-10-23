//! Incremental validation of text without retaining the complete input.
use std::io::Read;

#[derive(Default)]
pub struct Text {
    tail: [u8; 4],
    length: usize,
}

impl Text {
    pub fn push(&mut self, mut bytes: &[u8]) -> Result<(), String> {
        if bytes.contains(&0) {
            return Err("File contains binary data".into());
        }
        while self.length > 0 && !bytes.is_empty() {
            let take = (4 - self.length).min(bytes.len());
            self.tail[self.length..self.length + take].copy_from_slice(&bytes[..take]);
            self.length += take;
            bytes = &bytes[take..];
            match std::str::from_utf8(&self.tail[..self.length]) {
                Ok(_) => self.length = 0,
                Err(error) if error.error_len().is_none() => {
                    let valid = error.valid_up_to();
                    self.tail.copy_within(valid..self.length, 0);
                    self.length -= valid;
                }
                Err(_) => return Err("File is not UTF-8 text".into()),
            }
        }
        match std::str::from_utf8(bytes) {
            Ok(_) => Ok(()),
            Err(error) if error.error_len().is_none() => {
                let rest = &bytes[error.valid_up_to()..];
                self.tail[..rest.len()].copy_from_slice(rest);
                self.length = rest.len();
                Ok(())
            }
            Err(_) => Err("File is not UTF-8 text".into()),
        }
    }

    pub fn finish(self) -> Result<(), String> {
        if self.length == 0 {
            Ok(())
        } else {
            Err("File is not UTF-8 text".into())
        }
    }
}

pub fn is_text(mut reader: impl Read) -> std::io::Result<bool> {
    let mut text = Text::default();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(text.finish().is_ok());
        }
        if text.push(&buffer[..read]).is_err() {
            return Ok(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_every_boundary_and_rejects_truncated_or_binary_text() {
        let bytes = "aé日🦀z".as_bytes();
        for chunk in 1..bytes.len() {
            let mut text = Text::default();
            for part in bytes.chunks(chunk) {
                text.push(part).unwrap();
            }
            text.finish().unwrap();
        }
        for bytes in [b"\0".as_slice(), b"\xff", b"\xe2\x82", b"\xf0\x80\x80\x80"] {
            assert!(!is_text(bytes).unwrap());
        }
    }
}
