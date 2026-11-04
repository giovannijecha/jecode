//! The saved form of a file change, as sessions and receipts keep it.

use super::{Change, Entry, Kind};
use crate::json::Value;

impl Change {
    pub fn value(&self) -> Value {
        let mut value = Value::object([
            ("added", Value::number(self.added)),
            ("removed", Value::number(self.removed)),
            ("created", Value::Bool(self.created)),
            (
                "ending_change",
                self.ending_change
                    .as_ref()
                    .map_or(Value::Null, Value::string),
            ),
            (
                "final_newline",
                self.final_newline.map_or(Value::Null, Value::Bool),
            ),
            (
                "hunks",
                Value::Array(
                    self.hunks
                        .iter()
                        .map(|hunk| Value::Array(hunk.iter().map(Entry::value).collect()))
                        .collect(),
                ),
            ),
        ]);
        // Single-file changes keep their saved shape.
        if !self.files.is_empty()
            && let Value::Object(fields) = &mut value
        {
            fields.insert(
                "files".into(),
                Value::Array(
                    self.files
                        .iter()
                        .map(|(path, change)| {
                            Value::object([
                                ("path", Value::string(path)),
                                ("change", change.value()),
                            ])
                        })
                        .collect(),
                ),
            );
        }
        value
    }

    pub fn from_value(value: &Value) -> Result<Self, String> {
        let field = |key| {
            value
                .get(key)
                .ok_or_else(|| format!("Missing file change {key}"))
        };
        let added = field("added")?
            .as_usize()
            .ok_or("Invalid file change added")?;
        let removed = field("removed")?
            .as_usize()
            .ok_or("Invalid file change removed")?;
        let created = boolean(field("created")?, "created")?;
        let ending_change = optional_string(field("ending_change")?, "ending_change")?;
        let final_newline = optional_bool(field("final_newline")?, "final_newline")?;
        let hunks = field("hunks")?
            .as_array()
            .ok_or("Invalid file change hunks")?
            .iter()
            .map(|hunk| {
                hunk.as_array()
                    .ok_or("Invalid file change hunk")?
                    .iter()
                    .map(Entry::from_value)
                    .collect()
            })
            .collect::<Result<_, _>>()?;
        let files = match value.get("files") {
            None => Vec::new(),
            Some(files) => files
                .as_array()
                .ok_or("Invalid file change files")?
                .iter()
                .map(|file| {
                    let path = file
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or("Invalid file change path")?;
                    let change =
                        Self::from_value(file.get("change").ok_or("Missing file change")?)?;
                    Ok((path.to_owned(), change))
                })
                .collect::<Result<_, String>>()?,
        };
        Ok(Self {
            added,
            removed,
            created,
            hunks,
            ending_change,
            final_newline,
            files,
        })
    }
}

impl Entry {
    fn value(&self) -> Value {
        Value::object([
            (
                "kind",
                Value::string(match self.kind {
                    Kind::Context => "context",
                    Kind::Removed => "removed",
                    Kind::Added => "added",
                }),
            ),
            ("old_line", self.old_line.map_or(Value::Null, Value::number)),
            ("new_line", self.new_line.map_or(Value::Null, Value::number)),
            ("text", Value::string(&self.text)),
            ("no_newline", Value::Bool(self.no_newline)),
        ])
    }

    fn from_value(value: &Value) -> Result<Self, String> {
        let field = |key| {
            value
                .get(key)
                .ok_or_else(|| format!("Missing file change entry {key}"))
        };
        let kind = match field("kind")?.as_str() {
            Some("context") => Kind::Context,
            Some("removed") => Kind::Removed,
            Some("added") => Kind::Added,
            _ => return Err("Invalid file change entry kind".into()),
        };
        let old_line = optional_number(field("old_line")?, "old_line")?;
        let new_line = optional_number(field("new_line")?, "new_line")?;
        let text = field("text")?
            .as_str()
            .ok_or("Invalid file change entry text")?
            .into();
        let no_newline = boolean(field("no_newline")?, "no_newline")?;
        match kind {
            Kind::Context if old_line.is_none() || new_line.is_none() => {
                return Err("Invalid context coordinates".into());
            }
            Kind::Removed if old_line.is_none() || new_line.is_some() => {
                return Err("Invalid removed coordinates".into());
            }
            Kind::Added if old_line.is_some() || new_line.is_none() => {
                return Err("Invalid added coordinates".into());
            }
            _ => {}
        }
        Ok(Self {
            kind,
            old_line,
            new_line,
            text,
            no_newline,
        })
    }
}

fn boolean(value: &Value, field: &str) -> Result<bool, String> {
    match value {
        Value::Bool(value) => Ok(*value),
        _ => Err(format!("Invalid file change {field}")),
    }
}

fn optional_bool(value: &Value, field: &str) -> Result<Option<bool>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(value) => Ok(Some(*value)),
        _ => Err(format!("Invalid file change {field}")),
    }
}

fn optional_number(value: &Value, field: &str) -> Result<Option<usize>, String> {
    match value {
        Value::Null => Ok(None),
        _ => value
            .as_usize()
            .map(Some)
            .ok_or_else(|| format!("Invalid file change {field}")),
    }
}

fn optional_string(value: &Value, field: &str) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        _ => value
            .as_str()
            .map(|value| Some(value.into()))
            .ok_or_else(|| format!("Invalid file change {field}")),
    }
}
