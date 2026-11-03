use super::documents::{Document, Encoding, uri_to_path};
use crate::json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

/// Documents observed for this result, read and canonicalized once per path.
type Observed = HashMap<PathBuf, Option<Document>>;

pub(super) fn normalize(result: &Value, encoding: Encoding, source: &Document) -> Vec<Value> {
    let values = match result {
        Value::Array(values) => values.as_slice(),
        Value::Null => &[],
        other => std::slice::from_ref(other),
    };
    let mut observed = Observed::new();
    values
        .iter()
        .map(|value| location(value, encoding, source, &mut observed))
        .collect()
}

fn location(
    value: &Value,
    encoding: Encoding,
    source: &Document,
    observed: &mut Observed,
) -> Value {
    let (uri, range) = if value.get("targetUri").is_some() {
        (
            value.get("targetUri").and_then(Value::as_str),
            value
                .get("targetSelectionRange")
                .or_else(|| value.get("targetRange")),
        )
    } else {
        (value.get("uri").and_then(Value::as_str), value.get("range"))
    };
    let Some((uri, range)) = uri.zip(range) else {
        return Value::object([("resolved", Value::Bool(false)), ("raw", value.clone())]);
    };
    let path = uri_to_path(uri);
    let document = match &path {
        None => None,
        Some(_) if uri == source.uri => Some(source),
        Some(path) => observed
            .entry(path.clone())
            .or_insert_with(|| {
                if std::fs::canonicalize(path).ok().as_deref() == Some(source.path.as_path()) {
                    Some(source.clone())
                } else {
                    Document::read(path, "", 0).ok()
                }
            })
            .as_ref(),
    };
    let start = position(range.get("start"), document, encoding);
    let end = position(range.get("end"), document, encoding);
    Value::object([
        ("uri", Value::string(uri)),
        (
            "path",
            path.as_ref()
                .map_or(Value::Null, |p| Value::string(p.to_string_lossy())),
        ),
        ("start", start),
        ("end", end),
        (
            "resolved",
            Value::Bool(
                path.is_some() && range.get("start").is_some() && range.get("end").is_some(),
            ),
        ),
        (
            "mapping",
            Value::string(if document.is_some_and(|d| d.uri == source.uri) {
                "source_snapshot"
            } else {
                "observed_disk_or_raw"
            }),
        ),
    ])
}

fn position(value: Option<&Value>, document: Option<&Document>, encoding: Encoding) -> Value {
    let Some(value) = value else {
        return Value::Null;
    };
    let Some((line, character)) = value
        .get("line")
        .and_then(Value::as_usize)
        .zip(value.get("character").and_then(Value::as_usize))
    else {
        return Value::object([("raw", value.clone())]);
    };
    let column = document.and_then(|doc| doc.scalar_column(line, character, encoding));
    Value::object([
        ("line", Value::number(line + 1)),
        ("column", column.map_or(Value::Null, Value::number)),
        ("lsp_character", Value::number(character)),
    ])
}

pub(super) fn diagnostics(values: &[Value], document: &Document, encoding: Encoding) -> Vec<Value> {
    values
        .iter()
        .map(|item| {
            let range = item.get("range");
            Value::object([
                (
                    "message",
                    item.get("message").cloned().unwrap_or(Value::Null),
                ),
                (
                    "severity",
                    item.get("severity").cloned().unwrap_or(Value::Null),
                ),
                ("source", item.get("source").cloned().unwrap_or(Value::Null)),
                ("code", item.get("code").cloned().unwrap_or(Value::Null)),
                (
                    "start",
                    position(range.and_then(|v| v.get("start")), Some(document), encoding),
                ),
                (
                    "end",
                    position(range.and_then(|v| v.get("end")), Some(document), encoding),
                ),
                (
                    "raw",
                    if range.is_none() {
                        item.clone()
                    } else {
                        Value::Null
                    },
                ),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn non_file_location_is_preserved_as_unresolved() {
        let doc = Document {
            path: PathBuf::new(),
            uri: "file:///sample.rs".into(),
            language: "rust".into(),
            text: "x".into(),
            version: 1,
        };
        let result = Value::object([
            ("uri", Value::string("git:/virtual.rs")),
            (
                "range",
                Value::object([
                    (
                        "start",
                        Value::object([
                            ("line", Value::number(0)),
                            ("character", Value::number(0)),
                        ]),
                    ),
                    (
                        "end",
                        Value::object([
                            ("line", Value::number(0)),
                            ("character", Value::number(1)),
                        ]),
                    ),
                ]),
            ),
        ]);
        let normalized = normalize(&result, Encoding::Utf16, &doc);
        assert_eq!(normalized.len(), 1);
        assert_eq!(
            normalized[0].get("uri").and_then(Value::as_str),
            Some("git:/virtual.rs")
        );
        assert_eq!(normalized[0].get("path"), Some(&Value::Null));
    }

    fn at(uri: &str, line: usize, character: usize) -> Value {
        let point = || {
            Value::object([
                ("line", Value::number(line)),
                ("character", Value::number(character)),
            ])
        };
        Value::object([
            ("uri", Value::string(uri)),
            (
                "range",
                Value::object([("start", point()), ("end", point())]),
            ),
        ])
    }

    #[test]
    fn many_locations_in_one_file_map_from_one_observation() {
        let directory = crate::test_support::Directory::new();
        let source_path = directory.path().join("source.rs");
        std::fs::write(&source_path, "fn source() {}\n").unwrap();
        let source = Document::read(&source_path, "rust", 1).unwrap();
        let target = directory.path().join("target.rs");
        std::fs::write(&target, "a\u{1F980}b\n").unwrap();
        let uri = Document::read(&target, "", 0).unwrap().uri;
        let result = Value::Array((0..1000).map(|_| at(&uri, 0, 3)).collect());
        let normalized = normalize(&result, Encoding::Utf16, &source);
        assert_eq!(normalized.len(), 1000);
        for location in &normalized {
            let start = location.get("start").unwrap();
            assert_eq!(start.get("column").and_then(Value::as_usize), Some(3));
            assert_eq!(
                location.get("mapping").and_then(Value::as_str),
                Some("observed_disk_or_raw")
            );
        }
        let snapshot = normalize(&at(&source.uri, 0, 1), Encoding::Utf16, &source);
        assert_eq!(
            snapshot[0].get("mapping").and_then(Value::as_str),
            Some("source_snapshot")
        );
    }
}
