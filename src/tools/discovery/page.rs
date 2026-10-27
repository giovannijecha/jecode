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
            limit: optional_number(arguments, "limit", default, 1, maximum)?,
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
        let row = row()?;
        let size = row.encode().len();
        if self.bytes.saturating_add(size) > RESULT_BYTES {
            if self.rows.is_empty() {
                return Err("One result exceeds the output budget; reduce search context or narrow the path".into());
            }
            self.more = true;
            return Ok(false);
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
        while result.encode().len() > super::super::OUTPUT_LIMIT {
            let Value::Object(fields) = &mut result else {
                unreachable!()
            };
            let Some(Value::Array(rows)) = fields.get_mut(key) else {
                unreachable!()
            };
            if rows.len() <= 1 {
                return Err(
                    "Discovery result exceeds the output budget; narrow the path or reduce context"
                        .into(),
                );
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
