//! Bounded line previews and streaming literal matches, including long lines.
use crate::{cancel::Cancellation, json::Value};
use std::{
    collections::VecDeque,
    io::{BufRead, Read},
};

const PREVIEW: usize = 1024;
const LEADING: usize = 256;
const BLOCK: usize = 16 * 1024;

pub(super) struct Needle {
    bytes: Vec<u8>,
    prefix: Vec<usize>,
    sensitive: bool,
}

impl Needle {
    pub fn new(query: &str, sensitive: bool) -> Self {
        let bytes: Vec<_> = query
            .bytes()
            .map(|byte| {
                if sensitive {
                    byte
                } else {
                    byte.to_ascii_lowercase()
                }
            })
            .collect();
        let mut prefix = vec![0; bytes.len()];
        for index in 1..bytes.len() {
            let mut matched = prefix[index - 1];
            while matched > 0 && bytes[index] != bytes[matched] {
                matched = prefix[matched - 1];
            }
            if bytes[index] == bytes[matched] {
                matched += 1;
            }
            prefix[index] = matched;
        }
        Self {
            bytes,
            prefix,
            sensitive,
        }
    }

    fn advance(&self, matched: &mut usize, byte: u8) -> bool {
        let byte = if self.sensitive {
            byte
        } else {
            byte.to_ascii_lowercase()
        };
        while *matched > 0 && byte != self.bytes[*matched] {
            *matched = self.prefix[*matched - 1];
        }
        if byte == self.bytes[*matched] {
            *matched += 1;
        }
        *matched == self.bytes.len()
    }
}

#[derive(Clone)]
pub(super) struct Line {
    pub number: usize,
    pub text: String,
    pub truncated: bool,
    pub match_byte: Option<u64>,
}

impl Line {
    pub fn value(&self) -> Value {
        Value::object([
            ("line", Value::number(self.number)),
            ("text", Value::string(&self.text)),
            ("text_truncated", Value::Bool(self.truncated)),
        ])
    }
}

pub(super) enum Failure {
    Binary,
    Encoding,
    Error(String),
}

pub(super) struct Lines<R> {
    reader: R,
    offset: u64,
    number: usize,
    chunk: Vec<u8>,
}

impl<R: BufRead> Lines<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            offset: 0,
            number: 0,
            chunk: Vec::with_capacity(BLOCK),
        }
    }

    pub fn next(
        &mut self,
        needle: &Needle,
        cancellation: &Cancellation,
    ) -> Result<Option<Line>, Failure> {
        let start = self.offset;
        let mut length = 0usize;
        let mut matched = 0;
        let mut found = None;
        let mut leading = VecDeque::new();
        let mut head = Vec::new();
        let mut preview = Vec::new();
        let mut preview_start = 0;
        let mut utf8 = Vec::new();
        let mut ending = [0u8; 2];
        loop {
            if cancellation.requested() {
                return Err(Failure::Error("Operation cancelled".into()));
            }
            self.chunk.clear();
            let count = self
                .reader
                .by_ref()
                .take(BLOCK as u64)
                .read_until(b'\n', &mut self.chunk)
                .map_err(|error| Failure::Error(format!("Could not read file: {error}")))?;
            if count == 0 {
                break;
            }
            if self.chunk.contains(&0) {
                return Err(Failure::Binary);
            }
            utf8.extend_from_slice(&self.chunk);
            match std::str::from_utf8(&utf8) {
                Ok(_) => utf8.clear(),
                Err(error) if error.error_len().is_none() => {
                    utf8.drain(..error.valid_up_to());
                }
                Err(_) => return Err(Failure::Encoding),
            }
            for &byte in &self.chunk {
                ending = [ending[1], byte];
                length += 1;
                if head.len() < PREVIEW + 4 {
                    head.push(byte);
                }
                if found.is_none() {
                    leading.push_back(byte);
                    if leading.len() > needle.bytes.len().saturating_add(LEADING + 4) {
                        leading.pop_front();
                    }
                    if needle.advance(&mut matched, byte) {
                        found = Some(start + length as u64 - needle.bytes.len() as u64);
                        preview_start = length - leading.len();
                        preview.extend(leading.iter().take(PREVIEW + 4));
                    }
                } else if preview.len() < PREVIEW + 4 {
                    preview.push(byte);
                }
            }
            self.offset += count as u64;
            if self.chunk.last() == Some(&b'\n') {
                break;
            }
        }
        if !utf8.is_empty() {
            return Err(Failure::Encoding);
        }
        if length == 0 {
            return Ok(None);
        }
        self.number += 1;
        let bytes = if found.is_some() { &preview } else { &head };
        let mut first = 0;
        while bytes.get(first).is_some_and(|byte| byte & 0xc0 == 0x80) {
            first += 1;
        }
        let bytes = &bytes[first..];
        let valid =
            std::str::from_utf8(bytes).map_or_else(|error| error.valid_up_to(), |text| text.len());
        let text = std::str::from_utf8(&bytes[..valid]).unwrap();
        let omitted_start = if found.is_some() { preview_start } else { 0 };
        let newline = usize::from(ending[1] == b'\n') + usize::from(ending == [b'\r', b'\n']);
        let content_length = length.saturating_sub(newline);
        let end = text
            .floor_char_boundary(PREVIEW.min(content_length.saturating_sub(omitted_start + first)));
        let text = text[..end].to_owned();
        Ok(Some(Line {
            number: self.number,
            text,
            truncated: omitted_start + first > 0 || content_length > end,
            match_byte: found,
        }))
    }
}
