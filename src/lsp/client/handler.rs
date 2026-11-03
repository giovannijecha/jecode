use super::{Client, DiagnosticReport, WatchPattern};
use crate::json::Value;
use crate::lsp::documents::{canonical_uri, path_to_uri, uri_to_path};

impl Client {
    pub(super) fn handle(&mut self, message: Value) -> Result<(), String> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            if let Some(id) = message
                .get("id")
                .and_then(Value::as_usize)
                .map(|v| v as u64)
                && !self.cancelled.remove(&id)
                && self.pending.len() < 128
            {
                self.pending.insert(id, message);
            }
            return Ok(());
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if let Some(id) = message.get("id") {
            let result = match method {
                "workspace/configuration" => {
                    let items = params.get("items").and_then(Value::as_array).unwrap_or(&[]);
                    Ok(Value::Array(
                        items
                            .iter()
                            .map(|item| {
                                let section = item.get("section").and_then(Value::as_str);
                                section
                                    .and_then(|s| self.settings.get(s))
                                    .cloned()
                                    .unwrap_or_else(|| self.settings.clone())
                            })
                            .collect(),
                    ))
                }
                "workspace/workspaceFolders" => Ok(Value::Array(vec![Value::object([
                    ("uri", Value::string(path_to_uri(&self.root))),
                    (
                        "name",
                        Value::string(self.root.file_name().unwrap_or_default().to_string_lossy()),
                    ),
                ])])),
                "client/registerCapability" => {
                    let parsed = params
                        .get("registrations")
                        .and_then(Value::as_array)
                        .and_then(|values| {
                            values
                                .iter()
                                .map(|value| parse_registration(value, &self.root))
                                .collect::<Option<Vec<_>>>()
                        });
                    let parsed = parsed.filter(|parsed| {
                        let retained: usize = self
                            .registrations
                            .iter()
                            .filter(|(id, _)| !parsed.iter().any(|(new_id, _)| new_id == *id))
                            .map(|(_, watchers)| watchers.len())
                            .sum();
                        let retained_cost: usize = self
                            .registrations
                            .iter()
                            .filter(|(id, _)| !parsed.iter().any(|(new_id, _)| new_id == *id))
                            .flat_map(|(_, watchers)| watchers)
                            .map(|watch| watch.pattern.cost())
                            .sum();
                        let new_cost: usize = parsed
                            .iter()
                            .flat_map(|(_, watches)| watches)
                            .map(|watch| watch.pattern.cost())
                            .sum();
                        retained_cost + new_cost <= 64 * 1024
                            && retained
                                + parsed
                                    .iter()
                                    .map(|(_, watchers)| watchers.len())
                                    .sum::<usize>()
                                <= 512
                    });
                    if let Some(parsed) = parsed {
                        for (id, patterns) in parsed {
                            self.registrations.insert(id, patterns);
                        }
                        self.watch_initialized = false;
                        Ok(Value::Null)
                    } else {
                        Err(-32601)
                    }
                }
                "client/unregisterCapability" => {
                    let registrations = params
                        .get("unregisterations")
                        .or_else(|| params.get("unregistrations"))
                        .and_then(Value::as_array);
                    if registrations.is_some_and(|values| {
                        values.iter().all(|value| {
                            value.get("method").and_then(Value::as_str)
                                == Some("workspace/didChangeWatchedFiles")
                                && value.get("id").and_then(Value::as_str).is_some()
                        })
                    }) {
                        for registration in registrations.unwrap_or(&[]) {
                            if let Some(id) = registration.get("id").and_then(Value::as_str) {
                                self.registrations.remove(id);
                            }
                        }
                        Ok(Value::Null)
                    } else {
                        Err(-32601)
                    }
                }
                "window/workDoneProgress/create" => Ok(Value::Null),
                "workspace/diagnostic/refresh" => {
                    self.pull_diagnostics.clear();
                    Ok(Value::Null)
                }
                _ => Err(-32601),
            };
            let mut reply = vec![("jsonrpc", Value::string("2.0")), ("id", id.clone())];
            match result {
                Ok(value) => reply.push(("result", value)),
                Err(code) => reply.push((
                    "error",
                    Value::object([
                        ("code", Value::number(code)),
                        ("message", Value::string("Unsupported client request")),
                    ]),
                )),
            }
            self.wire.send(Value::Object(
                reply.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
            ))?;
        } else if method == "textDocument/publishDiagnostics" {
            if let (Some(uri), Some(values)) = (
                params.get("uri").and_then(Value::as_str),
                params.get("diagnostics").and_then(Value::as_array),
            ) {
                let version = params
                    .get("version")
                    .and_then(Value::as_usize)
                    .map(|v| v as i64);
                self.put_report(
                    canonical_uri(uri),
                    DiagnosticReport {
                        values: values.to_vec(),
                        version,
                        result_id: None,
                        generation: self.generation,
                        stale: false,
                    },
                    true,
                );
            }
        } else if method == "$/progress"
            && let (Some(token), Some(values)) = (
                params.get("token").and_then(Value::as_str),
                params.get("value").and_then(Value::as_array),
            )
            && let Some(partial) = self.partials.get_mut(token)
        {
            let size = values.iter().map(|v| v.encode().len()).sum::<usize>();
            if self.partial_bytes.saturating_add(size) > 8 * 1024 * 1024 {
                self.partial_truncated = true;
            } else {
                self.partial_bytes += size;
                partial.extend(values.iter().cloned());
            }
        }
        Ok(())
    }
}

fn parse_registration(
    value: &Value,
    root: &std::path::Path,
) -> Option<(String, Vec<WatchPattern>)> {
    if value.get("method")?.as_str()? != "workspace/didChangeWatchedFiles" {
        return None;
    }
    let id = value.get("id")?.as_str()?.to_owned();
    let watchers = value.get("registerOptions")?.get("watchers")?.as_array()?;
    if watchers.len() > 256 {
        return None;
    }
    let patterns = watchers
        .iter()
        .map(|watch| {
            let glob = watch.get("globPattern")?;
            let pattern = glob
                .as_str()
                .or_else(|| glob.get("pattern")?.as_str())?
                .to_owned();
            let normalized = pattern.replace('\\', "/");
            let normalized = normalized
                .strip_prefix("//?/UNC/")
                .map(|rest| format!("//{rest}"))
                .or_else(|| normalized.strip_prefix("//?/").map(str::to_owned))
                .unwrap_or(normalized);
            if normalized.split('/').any(|segment| segment == "..") {
                return None;
            }
            let absolute =
                normalized.starts_with('/') || normalized.as_bytes().get(1) == Some(&b':');
            let literal = (absolute && !normalized.contains(['*', '?', '[', '{']))
                .then(|| std::path::PathBuf::from(&normalized));
            let base = if let Some(uri) = glob.get("baseUri") {
                if absolute {
                    return None;
                }
                uri_to_path(uri.as_str().or_else(|| uri.get("uri")?.as_str())?)?
            } else if absolute {
                absolute_watch_base(&normalized)
            } else {
                root.to_path_buf()
            };
            let base = std::fs::canonicalize(&base).unwrap_or(base);
            if absolute && !base.is_absolute() {
                return None;
            }
            let kind = match watch.get("kind") {
                None => 7,
                Some(value) => value.as_usize()?,
            };
            if kind & !7 != 0 {
                return None;
            }
            Some(WatchPattern {
                pattern: crate::lsp::glob::Pattern::new(&pattern)?,
                base,
                kind,
                absolute,
                literal,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some((id, patterns))
}

fn absolute_watch_base(pattern: &str) -> std::path::PathBuf {
    let wildcard = pattern.find(['*', '?', '[', '{']).unwrap_or(pattern.len());
    let prefix = &pattern[..wildcard];
    let directory = prefix
        .rsplit_once('/')
        .map_or(prefix, |(directory, _)| directory);
    let directory = if directory.is_empty() {
        "/".to_owned()
    } else if directory.ends_with(':') {
        format!("{directory}/")
    } else {
        directory.to_owned()
    };
    directory.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lsp::documents::path_to_uri, test_support::Directory};

    fn registration(glob: Value) -> Value {
        Value::object([
            ("id", Value::string("watch")),
            ("method", Value::string("workspace/didChangeWatchedFiles")),
            (
                "registerOptions",
                Value::object([(
                    "watchers",
                    Value::Array(vec![Value::object([
                        ("globPattern", glob),
                        ("kind", Value::number(7)),
                    ])]),
                )]),
            ),
        ])
    }

    #[test]
    fn relative_watch_compiles_standard_patterns_and_workspace_folder_base() {
        let directory = Directory::new();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let glob = Value::object([
            (
                "baseUri",
                Value::object([
                    ("uri", Value::string(path_to_uri(&root))),
                    ("name", Value::string("fixture")),
                ]),
            ),
            (
                "pattern",
                Value::string("src/**/{file,other}[A-Z].{rs,txt}"),
            ),
        ]);
        let (_, watches) = parse_registration(&registration(glob), &root).unwrap();
        assert_eq!(watches[0].base, root);
        assert!(watches[0].matches(&root.join("src/fileA.rs")));
        assert!(watches[0].matches(&root.join("src/deep/otherB.txt")));
        assert!(!watches[0].matches(&root.join("src/file3.rs")));
    }

    #[test]
    fn absolute_watch_uses_the_literal_directory_as_its_scan_root() {
        let directory = Directory::new();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let pattern = format!("{}/**/watch-[0-9].{{json,toml}}", root.to_string_lossy());
        let (_, watches) =
            parse_registration(&registration(Value::string(pattern)), &root).unwrap();
        assert_eq!(watches[0].base, root);
        assert!(watches[0].absolute);
        assert!(watches[0].matches(&root.join("watch-3.json")));
        assert!(watches[0].matches(&root.join("deep/watch-9.toml")));
        assert!(!watches[0].matches(&root.join("watch-z.json")));
    }

    #[test]
    fn malformed_or_remote_watch_data_is_rejected_without_registration() {
        let directory = Directory::new();
        let root = directory.path();
        for glob in [
            Value::string("../*.rs"),
            Value::object([
                (
                    "baseUri",
                    Value::string("https://example.invalid/workspace"),
                ),
                ("pattern", Value::string("**/*.rs")),
            ]),
        ] {
            assert!(parse_registration(&registration(glob), root).is_none());
        }
        let mut data = registration(Value::string("**/*.rs"));
        if let Value::Object(fields) = &mut data {
            fields.insert("method".into(), Value::string("workspace/executeCommand"));
        }
        assert!(parse_registration(&data, root).is_none());
    }
}
