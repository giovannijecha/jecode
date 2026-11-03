use super::Client;
use crate::json::Value;
use crate::lsp::documents::Document;
use std::path::Path;

impl Client {
    pub fn reconcile(&mut self, path: &Path, language: &str) -> Result<Document, String> {
        let canonical = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
        let current = Document::read(&canonical, language, 0)?;
        self.synchronize(current)
    }

    /// Send a disk snapshot to the server. Errors here are protocol errors.
    fn synchronize(&mut self, mut current: Document) -> Result<Document, String> {
        let uri = current.uri.clone();
        let previous = self.docs.get(&uri).cloned();
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
        if previous.is_none() {
            if self.open_close {
                self.send_open(&current)?;
            }
            self.doc_order.push_back(uri.clone());
        } else {
            if self.sync_kind != 0 {
                self.send_change(&current)?;
            }
            self.send_save(&current)?;
            self.mark_stale(&uri);
        }
        self.docs.insert(uri, current.clone());
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
            if !self.docs.contains_key(&uri) {
                continue;
            }
            // A document that can no longer be read (deleted, replaced, not
            // UTF-8, too large, locked) is closed alone; a later query on it
            // reports the read error. Only protocol errors propagate.
            match Document::read(&path, &language, 0) {
                Ok(current) => {
                    self.synchronize(current)?;
                }
                Err(_) => self.close_document(&uri)?,
            }
        }
        Ok(())
    }

    fn close_document(&mut self, uri: &str) -> Result<(), String> {
        self.docs.remove(uri);
        self.doc_order.retain(|value| value != uri);
        self.pull_diagnostics.remove(uri);
        self.push_diagnostics.remove(uri);
        self.send_close(uri)
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
            if self.docs.contains_key(&oldest) {
                self.close_document(&oldest)?;
            }
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
                self.send_close(&document.uri)?;
                self.send_open(&document)?;
            } else {
                self.send_change(&document)?;
            }
            self.send_save(&document)?;
            self.mark_stale(&document.uri);
            self.docs.insert(document.uri.clone(), document);
        }
        Ok(())
    }

    fn mark_stale(&mut self, uri: &str) {
        for cache in [&mut self.pull_diagnostics, &mut self.push_diagnostics] {
            if let Some(report) = cache.get_mut(uri) {
                report.stale = true;
            }
        }
    }

    fn send_open(&mut self, document: &Document) -> Result<(), String> {
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
        )
    }

    fn send_close(&mut self, uri: &str) -> Result<(), String> {
        if !self.open_close {
            return Ok(());
        }
        self.notify(
            "textDocument/didClose",
            Value::object([("textDocument", Value::object([("uri", Value::string(uri))]))]),
        )
    }

    /// A rangeless change replaces the whole text. It is valid under Full and
    /// Incremental sync and leaves line-break interpretation to the server.
    fn send_change(&mut self, document: &Document) -> Result<(), String> {
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
                (
                    "contentChanges",
                    Value::Array(vec![Value::object([(
                        "text",
                        Value::string(&document.text),
                    )])]),
                ),
            ]),
        )
    }

    fn send_save(&mut self, document: &Document) -> Result<(), String> {
        if !self.save {
            return Ok(());
        }
        let mut params = std::collections::BTreeMap::from([(
            "textDocument".into(),
            Value::object([("uri", Value::string(&document.uri))]),
        )]);
        if self.save_include_text {
            params.insert("text".into(), Value::string(&document.text));
        }
        self.notify("textDocument/didSave", Value::Object(params))
    }
}
