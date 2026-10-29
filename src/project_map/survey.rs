//! A bounded structural survey built from the same filtered walk as discovery.
use crate::{cancel::Cancellation, json::Value, tools::discovery::walk::Walk};
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

mod outline;

const MAX_VISITED: usize = 100_000;
const MAX_ENTRIES: usize = 2_000;
const MAX_EXCERPT_FILES: usize = 80;

pub(super) struct Survey {
    pub scope: String,
    pub entries: Vec<Value>,
    pub files: usize,
    pub errors: Vec<Value>,
    pub errors_count: usize,
    pub skipped: Value,
    pub truncated: bool,
}

pub(super) fn collect(
    root: &Path,
    args: &Value,
    cancellation: &Cancellation,
) -> Result<Survey, String> {
    let query = match args.get("query") {
        None => "",
        Some(Value::String(query)) if !query.contains(['\r', '\n', '\0']) => query.as_str(),
        _ => return Err("query must be single-line literal text without NUL bytes".into()),
    };
    let query = query.to_lowercase();
    let mut walk = Walk::new_bounded(root, args, true, true, cancellation, MAX_VISITED)?;
    let scope = walk.scope.clone();
    let mut landmarks = Vec::new();
    let mut areas = BTreeMap::<String, usize>::new();
    let mut files = 0usize;
    let mut excerpt_files = 0usize;
    let mut truncated = false;
    while let Some(entry) = walk.next()? {
        if !walk.selected(&entry)? {
            continue;
        }
        match entry.kind {
            "directory" => {
                if entry.depth <= 2 {
                    areas.entry(entry.relative).or_default();
                }
            }
            "file" => {
                files += 1;
                for (index, _) in entry.relative.match_indices('/') {
                    if let Some(count) = areas.get_mut(&entry.relative[..index]) {
                        *count += 1;
                    }
                }
                if scope == entry.relative {
                    let result = outline::file(&entry.absolute, &entry.relative, cancellation)?;
                    truncated |= result.truncated;
                    for row in result.entries {
                        offer(&mut landmarks, row, &query, &mut truncated);
                    }
                } else if let Some(role) =
                    landmark_role(&entry.relative, scope != "." && entry.depth == 1)
                {
                    let source = if excerpt_files < MAX_EXCERPT_FILES {
                        excerpt_files += 1;
                        match excerpt(&entry.absolute) {
                            Ok(source) => source,
                            Err(error) => {
                                walk.error(&entry.relative, error);
                                None
                            }
                        }
                    } else {
                        None
                    };
                    let title = entry
                        .absolute
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(&entry.relative);
                    offer(
                        &mut landmarks,
                        row(
                            "landmark",
                            &entry.relative,
                            title,
                            role,
                            "filesystem",
                            source,
                        ),
                        &query,
                        &mut truncated,
                    );
                }
            }
            "symlink" => walk.skipped.links += 1,
            _ => walk.skipped.other += 1,
        }
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
    }
    let mut entries = Vec::new();
    for (area, count) in areas {
        let title = area.rsplit('/').next().unwrap_or(&area);
        let mut value = row(
            "area",
            &area,
            title,
            "Directory grouping (candidate)",
            "filesystem",
            None,
        );
        if let Value::Object(fields) = &mut value {
            fields.insert("file_count".into(), Value::number(count));
        }
        offer(&mut entries, value, &query, &mut truncated);
    }
    for landmark in landmarks {
        offer(&mut entries, landmark, "", &mut truncated);
    }
    entries.sort_by(|a, b| {
        let path_a = a.get("path").and_then(Value::as_str).unwrap_or("");
        let path_b = b.get("path").and_then(Value::as_str).unwrap_or("");
        path_a.cmp(path_b).then_with(|| {
            let title_a = a.get("title").and_then(Value::as_str).unwrap_or("");
            let title_b = b.get("title").and_then(Value::as_str).unwrap_or("");
            title_a.cmp(title_b)
        })
    });
    truncated |= walk.truncated;
    Ok(Survey {
        scope,
        entries,
        files,
        errors: walk.errors,
        errors_count: walk.errors_count,
        skipped: walk.skipped.value(),
        truncated,
    })
}

fn offer(entries: &mut Vec<Value>, row: Value, query: &str, truncated: &mut bool) {
    if !query.is_empty()
        && !["path", "title", "role"]
            .into_iter()
            .filter_map(|key| row.get(key).and_then(Value::as_str))
            .any(|text| text.to_lowercase().contains(query))
    {
        return;
    }
    if entries.len() < MAX_ENTRIES {
        entries.push(row);
    } else {
        *truncated = true;
    }
}

fn row(
    kind: &str,
    path: &str,
    title: &str,
    role: &str,
    origin: &str,
    source: Option<(usize, String)>,
) -> Value {
    let mut evidence = Value::object([("path", Value::string(path))]);
    if let Some((line, text)) = source
        && let Value::Object(fields) = &mut evidence
    {
        fields.insert("line".into(), Value::number(line));
        fields.insert("text".into(), Value::string(text));
    }
    Value::object([
        ("kind", Value::string(kind)),
        ("path", Value::string(path)),
        ("title", Value::string(title)),
        ("role", Value::string(role)),
        ("origin", Value::string(origin)),
        ("sources", Value::Array(vec![evidence])),
    ])
}

fn landmark_role(path: &str, scoped_child: bool) -> Option<&'static str> {
    let name = path.rsplit('/').next()?.to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "cargo.toml" | "package.json" | "pyproject.toml" | "go.mod"
    ) {
        Some("Package manifest (candidate)")
    } else if name.starts_with("readme") || name == "index.md" {
        Some("Project documentation (candidate)")
    } else if path.starts_with("docs/") && name.ends_with(".md") {
        Some("Documentation by path (candidate)")
    } else if name.starts_with("config")
        || name.ends_with(".config.js")
        || name.ends_with(".config.ts")
    {
        Some("Configuration (candidate)")
    } else if matches!(
        name.as_str(),
        "main.rs" | "lib.rs" | "main.py" | "app.py" | "index.ts" | "index.js"
    ) {
        Some("Entrypoint by filename (candidate)")
    } else if path.contains("/tests/")
        || name == "tests.rs"
        || name.ends_with("_test.rs")
        || name.ends_with("_tests.rs")
        || name.ends_with("_spec.rs")
    {
        Some("Tests by filename (candidate)")
    } else if name.ends_with(".rs")
        && (scoped_child || path.starts_with("src/") && path.matches('/').count() <= 1)
    {
        Some("Source module by path (candidate)")
    } else {
        None
    }
}

fn excerpt(path: &Path) -> Result<Option<(usize, String)>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(8192).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(None);
    };
    for (line, text) in text.lines().take(40).enumerate() {
        let trimmed = text.trim();
        if trimmed.starts_with("# ") || trimmed.starts_with("//! ") || trimmed.starts_with("/// ") {
            return Ok(Some((line + 1, clip(trimmed, 160))));
        }
    }
    Ok(None)
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::landmark_role;
    use crate::{
        cancel::Cancellation, json::Value, test_support::Directory, tools::discovery::walk::Walk,
    };
    use std::fs;

    #[test]
    fn filename_roles_remain_candidates() {
        assert_eq!(
            landmark_role("src/main.rs", false),
            Some("Entrypoint by filename (candidate)")
        );
        assert_eq!(
            landmark_role("src/engine.rs", false),
            Some("Source module by path (candidate)")
        );
        assert_eq!(landmark_role("assets/logo.png", false), None);
        assert_eq!(landmark_role("src/feature/worker.rs", false), None);
        assert_eq!(
            landmark_role("src/feature/worker.rs", true),
            Some("Source module by path (candidate)")
        );
    }

    #[test]
    fn scoped_source_directory_shows_its_immediate_rust_modules() {
        let directory = Directory::new();
        let root = fs::canonicalize(directory.path()).unwrap();
        let scope = root.join("src/feature");
        fs::create_dir_all(&scope).unwrap();
        for name in ["mod.rs", "settings.rs", "worker.rs"] {
            fs::write(
                scope.join(name),
                "//! Feature implementation.\npub fn run() {}\n",
            )
            .unwrap();
        }
        let result = super::collect(
            &root,
            &Value::object([
                ("path", Value::string("src/feature")),
                ("pattern", Value::string("**/*.rs")),
            ]),
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(result.files, 3);
        for name in ["mod.rs", "settings.rs", "worker.rs"] {
            let expected = format!("src/feature/{name}");
            assert!(
                result
                    .entries
                    .iter()
                    .any(|entry| entry.get("path").and_then(Value::as_str)
                        == Some(expected.as_str())),
                "scoped module {expected} missing: {:?}",
                result.entries
            );
        }
    }

    #[test]
    fn bounded_walk_stops_enumerating_a_large_single_directory() {
        let directory = Directory::new();
        let root = fs::canonicalize(directory.path()).unwrap();
        for index in 0..20 {
            fs::write(root.join(format!("file_{index:02}.txt")), "text").unwrap();
        }
        let cancellation = Cancellation::default();
        let args = Value::object([]);
        let mut walk = Walk::new_bounded(&root, &args, true, true, &cancellation, 3).unwrap();
        let mut paths = Vec::new();
        while let Some(entry) = walk.next().unwrap() {
            paths.push(entry.relative);
        }
        assert_eq!(paths.len(), 3);
        assert!(paths.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(walk.truncated);

        let mut full = Walk::new(&root, &args, true, true, &cancellation).unwrap();
        let mut full_count = 0;
        while full.next().unwrap().is_some() {
            full_count += 1;
        }
        assert_eq!(full_count, 20);
        assert!(!full.truncated);
    }

    #[test]
    fn hidden_candidates_consume_the_bounded_walk_budget() {
        let directory = Directory::new();
        let root = fs::canonicalize(directory.path()).unwrap();
        for index in 0..8 {
            fs::write(root.join(format!(".hidden_{index}")), "text").unwrap();
        }
        let cancellation = Cancellation::default();
        let args = Value::object([]);
        let mut walk = Walk::new_bounded(&root, &args, true, true, &cancellation, 3).unwrap();
        assert!(walk.next().unwrap().is_none());
        assert_eq!(walk.skipped.hidden, 3);
        assert!(walk.truncated);
    }
}
