use super::walk::Walk;
use crate::{json::Value, tools::optional_number};

const RESULT_BYTES: usize = 48 * 1024;

pub(crate) struct Page {
    rows: Vec<Value>,
    offset: usize,
    limit: usize,
    seen: usize,
    bytes: usize,
    more: bool,
}

impl Page {
    pub fn new(arguments: &Value, default: usize, maximum: usize) -> Result<Self, String> {
        Ok(Self {
            rows: Vec::new(),
            offset: optional_number(arguments, "offset", 1, 1, usize::MAX)?,
            limit: crate::tools::page_limit(arguments, "limit", default, maximum)?,
            seen: 0,
            bytes: 0,
            more: false,
        })
    }

    /// False means this row belongs to the next live page, not that it was lost.
    pub fn offer(&mut self, row: Value) -> Result<bool, String> {
        self.offer_with(|| Ok(row))
    }

    /// Inspect expensive row contents only when they reach the requested page.
    pub fn offer_with(
        &mut self,
        row: impl FnOnce() -> Result<Value, String>,
    ) -> Result<bool, String> {
        self.seen = self.seen.saturating_add(1);
        if self.seen < self.offset {
            return Ok(true);
        }
        if self.rows.len() >= self.limit {
            self.more = true;
            return Ok(false);
        }
        let mut row = row()?;
        let mut size = row.encode().len();
        if self.bytes.saturating_add(size) > RESULT_BYTES {
            if !self.rows.is_empty() {
                self.more = true;
                return Ok(false);
            }
            // A page always holds its first row, shortened if it must be.
            row = fit(row, RESULT_BYTES);
            size = row.encode().len();
        }
        self.bytes += size;
        self.rows.push(row);
        Ok(true)
    }

    pub fn finish<const N: usize>(
        self,
        key: &str,
        walk: &Walk<'_>,
        extra: [(&str, Value); N],
    ) -> Result<Value, String> {
        let mut fields = std::collections::BTreeMap::from([
            ("path".into(), Value::string(&walk.scope)),
            ("complete".into(), Value::Bool(walk.errors_count == 0)),
            ("skipped".into(), walk.skipped.value()),
            ("errors".into(), Value::Array(walk.errors.clone())),
            ("errors_count".into(), Value::number(walk.errors_count)),
            (
                "errors_truncated".into(),
                Value::Bool(walk.errors_count > walk.errors.len()),
            ),
        ]);
        fields.extend(extra.into_iter().map(|(key, value)| (key.into(), value)));
        self.finish_with(key, fields)
    }

    pub fn finish_with(
        self,
        key: &str,
        mut fields: std::collections::BTreeMap<String, Value>,
    ) -> Result<Value, String> {
        let count = self.rows.len();
        let incomplete = fields.get("complete") == Some(&Value::Bool(false));
        let truncated = fields.get("truncated") == Some(&Value::Bool(true));
        fields.extend([
            (key.into(), Value::Array(self.rows)),
            (format!("{key}_returned"), Value::number(count)),
            ("offset".into(), Value::number(self.offset)),
            (
                "next_offset".into(),
                if self.more {
                    Value::number(self.offset.saturating_add(count))
                } else {
                    Value::Null
                },
            ),
            ("truncated".into(), Value::Bool(self.more || truncated)),
            ("complete".into(), Value::Bool(!self.more && !incomplete)),
            (
                "output_limit_bytes".into(),
                Value::number(super::super::OUTPUT_LIMIT),
            ),
        ]);
        let mut result = Value::Object(fields);
        let mut refitted = false;
        while result.encode().len() > super::super::OUTPUT_LIMIT {
            let Value::Object(fields) = &mut result else {
                unreachable!()
            };
            let Some(Value::Array(rows)) = fields.get_mut(key) else {
                unreachable!()
            };
            if rows.len() <= 1 {
                // A lone row shrinks rather than failing the call; anything
                // still over is previewed in context with its history kept.
                match rows.first_mut() {
                    Some(row) if !refitted => {
                        refitted = true;
                        *row = fit(std::mem::replace(row, Value::Null), RESULT_BYTES / 4);
                        continue;
                    }
                    _ => break,
                }
            }
            rows.pop();
            let count = rows.len();
            fields.insert(format!("{key}_returned"), Value::number(count));
            fields.insert(
                "next_offset".into(),
                Value::number(self.offset.saturating_add(count)),
            );
            fields.insert("truncated".into(), Value::Bool(true));
            fields.insert("complete".into(), Value::Bool(false));
        }
        Ok(result)
    }
}

/// A row over `budget` keeps its shape with its long strings shortened and
/// `clipped: true`; its path and line still lead to the full text. Context and
/// other fields give way before the row's own `text`, so a match stays visible
/// whenever the rest can absorb the cut.
fn fit(mut row: Value, budget: usize) -> Value {
    let mut clipped = false;
    for own_text in [false, true] {
        let mut limit = 1024;
        while row.encode().len() > budget && limit > 16 {
            limit /= 2;
            clipped = true;
            match &mut row {
                Value::Object(fields) if !own_text => fields
                    .iter_mut()
                    .filter(|(key, _)| !matches!(key.as_str(), "path" | "text"))
                    .for_each(|(_, item)| shorten(item, limit)),
                _ => shorten(&mut row, limit),
            }
        }
    }
    if clipped && let Value::Object(fields) = &mut row {
        fields.insert("clipped".into(), Value::Bool(true));
    }
    row
}

fn shorten(value: &mut Value, limit: usize) {
    match value {
        Value::String(text) if text.len() > limit => {
            text.truncate(text.floor_char_boundary(limit));
            text.push('…');
        }
        Value::Array(items) => items.iter_mut().for_each(|item| shorten(item, limit)),
        Value::Object(fields) => {
            let mut text_shortened = false;
            for (key, item) in fields.iter_mut() {
                if key == "path" {
                    continue;
                }
                if key == "text" && matches!(item, Value::String(text) if text.len() > limit) {
                    text_shortened = true;
                }
                shorten(item, limit);
            }
            if text_shortened {
                fields.insert("text_truncated".into(), Value::Bool(true));
            }
        }
        _ => {}
    }
}
