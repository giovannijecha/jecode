use super::{Error, Value, parser::number_end};

pub(super) fn encode(value: &Value, limit: usize) -> Result<String, Error> {
    let mut writer = Writer {
        output: String::new(),
        limit,
    };
    writer.value(value, 0)?;
    Ok(writer.output)
}

struct Writer {
    output: String,
    limit: usize,
}

impl Writer {
    fn append(&mut self, text: &str) -> Result<(), Error> {
        if text.len() > self.limit.saturating_sub(self.output.len()) {
            return Err(Error::Limit);
        }
        self.output.push_str(text);
        Ok(())
    }
    fn string(&mut self, text: &str) -> Result<(), Error> {
        self.append("\"")?;
        let mut start = 0;
        for (at, ch) in text.char_indices() {
            if ch == '"' || ch == '\\' || ch <= '\x1f' {
                self.append(&text[start..at])?;
                match ch {
                    '"' => self.append("\\\"")?,
                    '\\' => self.append("\\\\")?,
                    '\n' => self.append("\\n")?,
                    '\r' => self.append("\\r")?,
                    '\t' => self.append("\\t")?,
                    _ => self.append(&format!("\\u{:04x}", ch as u32))?,
                }
                start = at + ch.len_utf8();
            }
        }
        self.append(&text[start..])?;
        self.append("\"")
    }
    fn value(&mut self, value: &Value, depth: usize) -> Result<(), Error> {
        if depth > 64 {
            return Err(Error::Limit);
        }
        match value {
            Value::Null => self.append("null"),
            Value::Bool(v) => self.append(if *v { "true" } else { "false" }),
            Value::Number(v) => {
                if number_end(v.as_bytes(), 0)? != v.len() {
                    return Err(Error::Syntax);
                }
                self.append(v)
            }
            Value::String(v) => self.string(v),
            Value::Array(items) => {
                self.append("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.append(",")?;
                    }
                    self.value(item, depth + 1)?;
                }
                self.append("]")
            }
            Value::Object(fields) => {
                self.append("{")?;
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.append(",")?;
                    }
                    self.string(key)?;
                    self.append(":")?;
                    self.value(value, depth + 1)?;
                }
                self.append("}")
            }
        }
    }
}
