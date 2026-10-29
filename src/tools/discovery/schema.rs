use crate::{json::Value, tools::schema::routine};

pub(crate) fn definitions() -> Vec<Value> {
    let common = || {
        std::collections::BTreeMap::from([
            ("path".into(), text()),
            ("pattern".into(), text()),
            ("include_hidden".into(), boolean()),
            ("include_ignored".into(), boolean()),
            ("offset".into(), integer(1, None)),
        ])
    };
    let mut list = common();
    list.insert("recursive".into(), boolean());
    list.insert("limit".into(), integer(1, Some(2000)));
    let mut search = common();
    search.insert("query".into(), text());
    search.insert("case_sensitive".into(), boolean());
    search.insert("context".into(), integer(0, Some(10)));
    search.insert("limit".into(), integer(1, Some(1000)));
    vec![
        routine(
            "list",
            "List a project directory: path defaults '.', recursive defaults false (true gives a tree). Entries have workspace-relative path, type, depth (children start at 1) and file size. Hidden and Git-ignored entries default off; include_hidden/include_ignored opt in separately. Explicit path bypasses its own exclusion; links are listed without traversal. pattern is a glob: *, ?, **, [classes]; no slash matches a basename, otherwise a workspace-relative path. Matching folds ASCII case on Windows; directories are still explored. Git metadata needs include_ignored. offset defaults 1; limit defaults 200 (max 2000). Output <=64 KiB; continue with next_offset and identical arguments. Pages use live files; restart after changes. complete:false means more results or errors; skipped counts exclusions. Local .gitignore and info/exclude apply, not global Git excludes.",
            Value::Object(list),
            &[],
        ),
        routine(
            "search",
            "Search literal single-line query in UTF-8 project files; no regex. path defaults '.' (recursive), or select one file. Results: workspace-relative path, 1-based line, text, before/after context and byte_offset of the first match; one result per matching line. case_sensitive defaults true; false ignores ASCII case. context defaults 2 (0-10). Long lines stream; text_truncated marks previews centered on the first match; recover with read(path,byte_offset). Filtering and pattern follow list. Explicit path bypasses its own exclusion; implicit links are skipped. Non-text files are counted as skipped; invalid tails after valid lines and read failures yield errors, preserving earlier matches. offset defaults 1; limit defaults 100 (max 1000); output <=64 KiB. Follow next_offset with identical arguments; restart after file changes. complete:false means pagination or errors; check skipped before concluding absence.",
            Value::Object(search),
            &["query"],
        ),
    ]
}

fn text() -> Value {
    Value::object([("type", Value::string("string"))])
}

fn boolean() -> Value {
    Value::object([("type", Value::string("boolean"))])
}

fn integer(minimum: usize, maximum: Option<usize>) -> Value {
    let mut fields = std::collections::BTreeMap::from([
        ("type".into(), Value::string("integer")),
        ("minimum".into(), Value::number(minimum)),
    ]);
    if let Some(maximum) = maximum {
        fields.insert("maximum".into(), Value::number(maximum));
    }
    Value::Object(fields)
}
