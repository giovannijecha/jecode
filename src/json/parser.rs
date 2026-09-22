use super::{Error, Limits, Value, strings};
use std::collections::{BTreeMap, btree_map::Entry};

pub(super) fn parse(text: &str, limits: Limits) -> Result<Value, Error> {
    if text.len() > limits.bytes {
        return Err(Error::Limit);
    }
    let mut parser = Parser {
        text,
        at: 0,
        remaining: limits.nodes,
        depth: limits.depth.min(64),
    };
    let value = parser.value(0)?;
    parser.space();
    if parser.at != text.len() {
        return Err(Error::Syntax);
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
    remaining: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }
    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.at += 1;
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > self.depth || self.remaining == 0 {
            return Err(Error::Limit);
        }
        self.remaining -= 1;
        self.space();
        match self.peek() {
            Some(b'n') => self.literal("null", Value::Null),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'"') => strings::read(self.text, &mut self.at).map(Value::String),
            Some(b'[') => self.array(depth),
            Some(b'{') => self.object(depth),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.at;
                self.at = number_end(self.text.as_bytes(), start)?;
                Ok(Value::Number(self.text[start..self.at].to_owned()))
            }
            _ => Err(Error::Syntax),
        }
    }
    fn literal(&mut self, text: &str, value: Value) -> Result<Value, Error> {
        if !self.text[self.at..].starts_with(text) {
            return Err(Error::Syntax);
        }
        self.at += text.len();
        Ok(value)
    }
    fn array(&mut self, depth: usize) -> Result<Value, Error> {
        self.at += 1;
        self.space();
        let mut items = Vec::new();
        if self.take(b']') {
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.space();
            if self.take(b']') {
                return Ok(Value::Array(items));
            }
            if !self.take(b',') {
                return Err(Error::Syntax);
            }
        }
    }
    fn object(&mut self, depth: usize) -> Result<Value, Error> {
        self.at += 1;
        self.space();
        let mut fields = BTreeMap::new();
        if self.take(b'}') {
            return Ok(Value::Object(fields));
        }
        loop {
            self.space();
            let key = strings::read(self.text, &mut self.at)?;
            self.space();
            if !self.take(b':') {
                return Err(Error::Syntax);
            }
            match fields.entry(key) {
                Entry::Occupied(_) => return Err(Error::DuplicateKey),
                Entry::Vacant(entry) => {
                    entry.insert(self.value(depth + 1)?);
                }
            }
            self.space();
            if self.take(b'}') {
                return Ok(Value::Object(fields));
            }
            if !self.take(b',') {
                return Err(Error::Syntax);
            }
        }
    }
}

pub(super) fn number_end(bytes: &[u8], mut at: usize) -> Result<usize, Error> {
    if bytes.get(at) == Some(&b'-') {
        at += 1;
    }
    match bytes.get(at) {
        Some(b'0') => at += 1,
        Some(b'1'..=b'9') => {
            at += 1;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
        }
        _ => return Err(Error::Syntax),
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return Err(Error::Syntax);
        }
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return Err(Error::Syntax);
        }
    }
    Ok(at)
}
