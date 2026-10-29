use crate::{json::Value, tools::schema::routine};

pub(super) fn definitions() -> Vec<Value> {
    let text = || Value::object([("type", Value::string("string"))]);
    let integer = |maximum: Option<usize>| {
        let mut fields = std::collections::BTreeMap::from([
            ("type".into(), Value::string("integer")),
            ("minimum".into(), Value::number(1)),
        ]);
        if let Some(maximum) = maximum {
            fields.insert("maximum".into(), Value::number(maximum));
        }
        Value::Object(fields)
    };
    vec![routine(
        "map",
        "Navigate a project. overview (default) returns live areas, candidate entrypoints/config/tests/docs and saved model notes with source references. path defaults '.'; one Rust/Markdown file gives lexical declarations/headings, not a call graph. query filters path/title/role/summary by case-insensitive literal text; pattern and hidden/ignored options follow list. offset defaults 1, limit 100 (max 1000). Output <=64 KiB; follow next_offset with identical arguments. inventory_truncated means narrow the path/query; complete:false also covers errors. Inventory caps: 100000 candidates, 2000 rows; headers 80 x 8 KiB, focused files 4 MiB. remember requires title, summary, sources:[{path,line?}]; optional id updates a note. Sources are workspace files with whole-content fingerprints and optional 1-based line excerpts. Notes are shared across conversations in user storage. overview checks only requested notes: current/changed/missing/unverified. Freshness covers referenced content, not semantic correctness or unreferenced dependencies. Fingerprint budget: 64 MiB/call; excess is unverified (remember fails). forget requires id. No startup map scan or injection. Only overview is read-only.",
        Value::object([
            (
                "action",
                Value::object([
                    ("type", Value::string("string")),
                    (
                        "enum",
                        Value::Array(
                            ["overview", "remember", "forget"]
                                .map(Value::string)
                                .to_vec(),
                        ),
                    ),
                ]),
            ),
            ("path", text()),
            ("query", text()),
            ("pattern", text()),
            (
                "include_hidden",
                Value::object([("type", Value::string("boolean"))]),
            ),
            (
                "include_ignored",
                Value::object([("type", Value::string("boolean"))]),
            ),
            ("offset", integer(None)),
            ("limit", integer(Some(1000))),
            ("id", text()),
            ("title", text()),
            ("summary", text()),
            (
                "sources",
                Value::object([
                    ("type", Value::string("array")),
                    ("minItems", Value::number(1)),
                    ("maxItems", Value::number(16)),
                    (
                        "items",
                        Value::object([
                            ("type", Value::string("object")),
                            (
                                "properties",
                                Value::object([("path", text()), ("line", integer(None))]),
                            ),
                            ("required", Value::Array(vec![Value::string("path")])),
                            ("additionalProperties", Value::Bool(false)),
                        ]),
                    ),
                ]),
            ),
        ]),
        &[],
    )]
}
