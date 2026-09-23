//! Bounded, strict JSON for provider wire data. Numbers retain their spelling.
//! Duplicate keys and unpaired UTF-16 escapes are rejected, never repaired.

mod parser;
mod strings;
mod writer;

use std::{collections::BTreeMap, fmt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(fields) => fields.get(key),
            _ => None,
        }
    }
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }
    pub fn array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }
    pub fn unsigned(&self) -> Option<u64> {
        match self {
            Self::Number(number) if number.bytes().all(|b| b.is_ascii_digit()) => {
                number.parse().ok()
            }
            _ => None,
        }
    }
}

pub fn object<'a>(pairs: impl IntoIterator<Item = (&'a str, Value)>) -> Value {
    Value::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

#[derive(Clone, Copy)]
pub struct Limits {
    pub bytes: usize,
    pub nodes: usize,
    pub depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: 2 * 1024 * 1024,
            nodes: 65_536,
            depth: 64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Syntax,
    Unicode,
    DuplicateKey,
    Limit,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Syntax => "invalid JSON syntax",
            Self::Unicode => "invalid JSON Unicode escape",
            Self::DuplicateKey => "duplicate JSON object key",
            Self::Limit => "JSON exceeds its configured limit",
        })
    }
}
impl std::error::Error for Error {}

pub fn parse(text: &str, limits: Limits) -> Result<Value, Error> {
    parser::parse(text, limits)
}

pub fn encode(value: &Value, max_bytes: usize) -> Result<String, Error> {
    writer::encode(value, max_bytes)
}
