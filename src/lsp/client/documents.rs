use super::{Client, end_position};
use crate::json::Value;
use crate::lsp::documents::{Document, path_to_uri};
use std::path::Path;

impl Client {
    pub fn reconcile(&mut self, path: &Path, language: &str) -> Result<Document, String> {
        let canonical = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
        let uri = path_to_uri(&canonical);
        let previous = self.docs.get(&uri).cloned();
        let mut current = Document::read(&canonical, language, 0)?;
        if let Some(old) = &previous
            && old.text == current.text
        {
            return Ok(old.clone());
        }
        if self.next_document_version > i32::MAX as i64 {
            return Err("LSP document version exhausted".into());
        }
        current.version = self.next_document_version;
        self.next_document_version += 1;
        self.evict_documents(&uri, current.text.len())?;
        match previous {
            None => {
                if self.open_close {
                    self.notify(
                        "textDocument/didOpen",
                        Value::object([(
                            "textDocument",
                            Value::object([
                                ("uri", Value::string(&current.uri)),
                                ("languageId", Value::string(language)),
                                ("version", Value::number(current.version)),
                                ("text", Value::string(&current.text)),
                            ]),
                        )]),
                    )?;
                }
                self.docs.insert(uri, current.clone());
                self.doc_order.push_back(current.uri.clone());
            }
            Some(old) if old.text != current.text => {
                let change = if self.sync_kind == 2 {
                    Value::object([
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
                                ("end", end_position(&old, self.encoding)),
                            ]),
                        ),
                        ("text", Value::string(&current.text)),
                    ])
                } else {
                    Value::object([("text", Value::string(&current.text))])
                };
                if self.sync_kind != 0 {
                    self.notify(
                        "textDocument/didChange",
                        Value::object([
                            (
                                "textDocument",
                                Value::object([
                                    ("uri", Value::string(&uri)),
                                    ("version", Value::number(current.version)),
                                ]),
                            ),
                            ("contentChanges", Value::Array(vec![change])),
                        ]),
                    )?;
                }
                if self.save {
                    let mut params = std::collections::BTreeMap::from([(
                        "textDocument".into(),
                        Value::object([("uri", Value::string(&uri))]),
                    )]);
                    if self.save_include_text {
                        params.insert("text".into(), Value::string(&current.text));
                    }
                    self.notify("textDocument/didSave", Value::Object(params))?;
                }
                self.docs.insert(uri.clone(), current.clone());
                for cache in [&mut self.pull_diagnostics, &mut self.push_diagnostics] {
                    if let Some(report) = cache.get_mut(&uri) {
                        report.stale = true;
                    }
                }
            }
            Some(_) => unreachable!("unchanged document returned above"),
        }
        Ok(current)
    }

    pub fn poll_open_documents(&mut self) -> Result<(), String> {
        self.poll_watches()?;
        let docs = self
            .docs
            .values()
            .map(|d| (d.path.clone(), d.language.clone(), d.uri.clone()))
            .collect::<Vec<_>>();
        for (path, language, uri) in docs {
            if path.exists() {
                self.reconcile(&path, &language)?;
            } else {
                if self.open_close {
                    self.notify(
                        "textDocument/didClose",
                        Value::object([(
                            "textDocument",
                            Value::object([("uri", Value::string(&uri))]),
                        )]),
                    )?;
                }
                self.docs.remove(&uri);
                self.doc_order.retain(|value| value != &uri);
                self.pull_diagnostics.remove(&uri);
                self.push_diagnostics.remove(&uri);
            }
        }
        Ok(())
    }

    fn evict_documents(&mut self, keep: &str, incoming_bytes: usize) -> Result<(), String> {
        while self.docs.len() >= 128 && !self.docs.contains_key(keep)
            || self
                .docs
                .values()
                .filter(|doc| doc.uri != keep)
                .map(|doc| doc.text.len())
                .sum::<usize>()
                + incoming_bytes
                > 32 * 1024 * 1024
        {
            let Some(oldest) = self.doc_order.pop_front() else {
                break;
            };
            if oldest == keep {
                self.doc_order.push_back(oldest);
                break;
            }
            if self.docs.remove(&oldest).is_none() {
                continue;
            }
            if self.open_close {
                self.notify(
                    "textDocument/didClose",
                    Value::object([(
                        "textDocument",
                        Value::object([("uri", Value::string(&oldest))]),
                    )]),
                )?;
            }
            self.pull_diagnostics.remove(&oldest);
            self.push_diagnostics.remove(&oldest);
        }
        Ok(())
    }

    pub(super) fn resync_managed_documents(&mut self) -> Result<(), String> {
        if self.sync_kind == 0 && !self.open_close {
            self.sync_gap = true;
            return Ok(());
        }
        let docs = self.docs.values().cloned().collect::<Vec<_>>();
        for mut document in docs {
            if self.next_document_version > i32::MAX as i64 {
                self.sync_gap = true;
                return Ok(());
            }
            document.version = self.next_document_version;
            self.next_document_version += 1;
            if self.open_close {
                self.notify(
                    "textDocument/didClose",
                    Value::object([(
                        "textDocument",
                        Value::object([("uri", Value::string(&document.uri))]),
                    )]),
                )?;
                self.notify(
                    "textDocument/didOpen",
                    Value::object([(
                        "textDocument",
                        Value::object([
                            ("uri", Value::string(&document.uri)),
                            ("languageId", Value::string(&document.language)),
                            ("version", Value::number(document.version)),
                            ("text", Value::string(&document.text)),
                        ]),
                    )]),
                )?;
            } else {
                let change = if self.sync_kind == 2 {
                    Value::object([
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
                                ("end", end_position(&document, self.encoding)),
                            ]),
                        ),
                        ("text", Value::string(&document.text)),
                    ])
                } else {
                    Value::object([("text", Value::string(&document.text))])
                };
                self.notify(
                    "textDocument/didChange",
                    Value::object([
                        (
                            "textDocument",
                            Value::object([
                                ("uri", Value::string(&document.uri)),
                                ("version", Value::number(document.version)),
                            ]),
                        ),
                        ("contentChanges", Value::Array(vec![change])),
                    ]),
                )?;
            }
            if self.save {
                let mut params = std::collections::BTreeMap::from([(
                    "textDocument".into(),
                    Value::object([("uri", Value::string(&document.uri))]),
                )]);
                if self.save_include_text {
                    params.insert("text".into(), Value::string(&document.text));
                }
                self.notify("textDocument/didSave", Value::Object(params))?;
            }
            for cache in [&mut self.pull_diagnostics, &mut self.push_diagnostics] {
                if let Some(report) = cache.get_mut(&document.uri) {
                    report.stale = true;
                }
            }
            self.docs.insert(document.uri.clone(), document);
        }
        Ok(())
    }
}
