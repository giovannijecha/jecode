use crate::json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Encoding {
    Utf8,
    Utf16,
    Utf32,
}

#[derive(Clone)]
pub(super) struct Document {
    pub path: PathBuf,
    pub uri: String,
    pub language: String,
    pub text: String,
    pub version: i64,
}

impl Document {
    pub fn read(path: &Path, language: &str, version: i64) -> Result<Self, String> {
        let meta = fs::metadata(path).map_err(|e| format!("Cannot read LSP document: {e}"))?;
        if !meta.is_file() {
            return Err("LSP document must be a file".into());
        }
        let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("Cannot read UTF-8 LSP document: {e}"))?;
        Ok(Self {
            uri: path_to_uri(&path),
            path,
            language: language.into(),
            text,
            version,
        })
    }

    pub fn model_position(
        &self,
        line: usize,
        column: usize,
        encoding: Encoding,
    ) -> Result<Value, String> {
        if line == 0 || column == 0 {
            return Err("LSP line and column are one-based".into());
        }
        let content = self
            .line(line - 1)
            .ok_or("LSP line is outside the document")?;
        let prefix: String = content.chars().take(column - 1).collect();
        if prefix.chars().count() != column - 1 {
            return Err("LSP column is outside the line".into());
        }
        let offset = match encoding {
            Encoding::Utf8 => prefix.len(),
            Encoding::Utf16 => prefix.encode_utf16().count(),
            Encoding::Utf32 => prefix.chars().count(),
        };
        Ok(Value::object([
            ("line", Value::number(line - 1)),
            ("character", Value::number(offset)),
        ]))
    }

    pub fn scalar_column(&self, line: usize, offset: usize, encoding: Encoding) -> Option<usize> {
        let text = self.line(line)?;
        let mut position = 0;
        for (index, ch) in text.chars().enumerate() {
            if position == offset {
                return Some(index + 1);
            }
            position += match encoding {
                Encoding::Utf8 => ch.len_utf8(),
                Encoding::Utf16 => ch.len_utf16(),
                Encoding::Utf32 => 1,
            };
            if position > offset {
                return None;
            }
        }
        (position == offset).then_some(text.chars().count() + 1)
    }

    pub fn line(&self, index: usize) -> Option<&str> {
        if self.text.is_empty() {
            return (index == 0).then_some("");
        }
        let mut start = 0;
        for (current, chunk) in self.text.split_inclusive('\n').enumerate() {
            if current == index {
                return Some(chunk.trim_end_matches('\n').trim_end_matches('\r'));
            }
            start += chunk.len();
        }
        if start == self.text.len()
            && self.text.ends_with('\n')
            && index == self.text.bytes().filter(|b| *b == b'\n').count()
        {
            return Some("");
        }
        None
    }
}

pub(super) fn path_to_uri(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    let path = path
        .strip_prefix(r"\\?\UNC\")
        .map(|tail| format!(r"\\{tail}"))
        .or_else(|| path.strip_prefix(r"\\?\").map(str::to_owned))
        .unwrap_or_else(|| path.into_owned());
    let path = path.replace('\\', "/");
    let mut uri = if path.starts_with("//") {
        "file:".to_owned()
    } else if path.as_bytes().get(1) == Some(&b':') {
        "file:///".to_owned()
    } else {
        "file://".to_owned()
    };
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

pub(super) fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let mut body = uri.strip_prefix("file://")?;
    if body
        .get(..9)
        .is_some_and(|host| host.eq_ignore_ascii_case("localhost"))
        && body.as_bytes().get(9) == Some(&b'/')
    {
        body = &body[9..];
    }
    let bytes = body.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = bytes.get(i + 1)?.to_ascii_uppercase();
            let lo = bytes.get(i + 2)?.to_ascii_uppercase();
            let digit = |b| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            result.push(digit(hi)? * 16 + digit(lo)?);
            i += 3;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(result).ok()?;
    #[cfg(windows)]
    let decoded = {
        let d = if decoded.as_bytes().get(2) == Some(&b':') {
            &decoded[1..]
        } else {
            &decoded
        };
        if d.as_bytes().get(1) == Some(&b':') {
            d.replace('/', "\\")
        } else if !d.starts_with('/') {
            format!("\\\\{}", d.replace('/', "\\"))
        } else {
            d.replace('/', "\\")
        }
    };
    #[cfg(not(windows))]
    let decoded = if decoded.starts_with('/') {
        decoded
    } else {
        format!("//{decoded}")
    };
    Some(PathBuf::from(decoded))
}

pub(super) fn canonical_uri(uri: &str) -> String {
    uri_to_path(uri)
        .and_then(|path| fs::canonicalize(path).ok())
        .map(|path| path_to_uri(&path))
        .unwrap_or_else(|| uri.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_positions_cover_crlf_and_non_bmp() {
        let doc = Document {
            path: PathBuf::new(),
            uri: String::new(),
            language: String::new(),
            text: "a😀b\r\nx".into(),
            version: 1,
        };
        assert_eq!(
            doc.model_position(1, 3, Encoding::Utf16)
                .unwrap()
                .get("character")
                .and_then(Value::as_usize),
            Some(3)
        );
        assert_eq!(doc.scalar_column(0, 3, Encoding::Utf16), Some(3));
        assert_eq!(doc.line(0), Some("a😀b"));
        assert_eq!(doc.line(1), Some("x"));
    }
    #[test]
    fn uri_round_trip() {
        let path = PathBuf::from(if cfg!(windows) {
            r"C:\tmp\é space.rs"
        } else {
            "/tmp/é space.rs"
        });
        assert_eq!(uri_to_path(&path_to_uri(&path)), Some(path));
        #[cfg(windows)]
        assert_eq!(
            uri_to_path("file://localhost/C:/tmp/a.rs"),
            Some(PathBuf::from(r"C:\tmp\a.rs"))
        );
        #[cfg(not(windows))]
        assert_eq!(
            uri_to_path("file://localhost/tmp/a.rs"),
            Some(PathBuf::from("/tmp/a.rs"))
        );
    }
}
