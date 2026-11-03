use super::Profile;
use super::documents::{Document, Encoding, path_to_uri};
use super::transport::Transport;
use crate::cancel::Cancellation;
use crate::json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub(super) struct DiagnosticReport {
    pub values: Vec<Value>,
    pub version: Option<i64>,
    pub result_id: Option<String>,
    pub generation: u64,
    pub stale: bool,
}

pub(super) struct Client {
    wire: Transport,
    pub capabilities: Value,
    pub encoding: Encoding,
    pub sync_kind: usize,
    pub open_close: bool,
    pub save: bool,
    pub save_include_text: bool,
    pub docs: HashMap<String, Document>,
    doc_order: VecDeque<String>,
    pub pull_diagnostics: HashMap<String, DiagnosticReport>,
    pub push_diagnostics: HashMap<String, DiagnosticReport>,
    report_ledger: reports::Ledger,
    pub generation: u64,
    next_document_version: i64,
    next_id: u64,
    pending: HashMap<u64, Value>,
    cancelled: HashSet<u64>,
    partials: HashMap<String, Vec<Value>>,
    partial_bytes: usize,
    pub partial_truncated: bool,
    registrations: HashMap<String, Vec<WatchPattern>>,
    observed_extensions: Vec<String>,
    observed_markers: Vec<String>,
    watched: HashMap<PathBuf, watches::Stamp>,
    /// False until a scan completes after startup or a re-registration; that
    /// scan treats paths not tracked before as a baseline, not as creations.
    watch_initialized: bool,
    pub sync_gap: bool,
    settings: Value,
    root: PathBuf,
}

struct WatchPattern {
    pattern: crate::lsp::glob::Pattern,
    base: PathBuf,
    kind: usize,
    absolute: bool,
    literal: Option<PathBuf>,
}

impl WatchPattern {
    fn matches(&self, path: &Path) -> bool {
        if self.absolute {
            self.pattern.matches(&path.to_string_lossy())
        } else {
            path.strip_prefix(&self.base)
                .ok()
                .is_some_and(|path| self.pattern.matches(&path.to_string_lossy()))
        }
    }
}

mod documents;
mod handler;
mod reports;
#[cfg(test)]
mod tests;
mod watches;

impl Client {
    pub fn pid(&self) -> u32 {
        self.wire.id()
    }
    pub fn start(
        profile: &Profile,
        root: &Path,
        cancel: &Cancellation,
        stop: &Cancellation,
        generation: u64,
    ) -> Result<Self, String> {
        if cancel.requested() {
            return Err("LSP request cancelled before startup".into());
        }
        if stop.requested() {
            return Err("LSP settings changed before startup".into());
        }
        let executable = super::executable::resolve(profile, root)?;
        let mut command = Command::new(executable);
        command.args(&profile.arguments).current_dir(root);
        for (key, value) in &profile.environment {
            command.env(key, value);
        }
        let wire = Transport::spawn(&mut command, cancel)?;
        let mut client = Self {
            wire,
            capabilities: Value::Null,
            encoding: Encoding::Utf16,
            sync_kind: 0,
            open_close: false,
            save: false,
            save_include_text: false,
            docs: HashMap::new(),
            doc_order: VecDeque::new(),
            pull_diagnostics: HashMap::new(),
            push_diagnostics: HashMap::new(),
            report_ledger: reports::Ledger::default(),
            generation,
            next_document_version: 1,
            next_id: 1,
            pending: HashMap::new(),
            cancelled: HashSet::new(),
            partials: HashMap::new(),
            partial_bytes: 0,
            partial_truncated: false,
            registrations: HashMap::new(),
            observed_extensions: profile.extensions.keys().cloned().collect(),
            observed_markers: profile.root_markers.clone(),
            watched: HashMap::new(),
            watch_initialized: false,
            sync_gap: false,
            settings: profile.settings.clone(),
            root: root.to_path_buf(),
        };
        let root_uri = path_to_uri(root);
        let params = Value::object([
            ("processId", Value::number(std::process::id())),
            ("rootUri", Value::string(&root_uri)),
            ("rootPath", Value::string(root.to_string_lossy())),
            (
                "workspaceFolders",
                Value::Array(vec![Value::object([
                    ("uri", Value::string(&root_uri)),
                    (
                        "name",
                        Value::string(root.file_name().unwrap_or_default().to_string_lossy()),
                    ),
                ])]),
            ),
            (
                "initializationOptions",
                profile.initialization_options.clone(),
            ),
            ("capabilities", client_capabilities()),
            (
                "clientInfo",
                Value::object([("name", Value::string("Jecode"))]),
            ),
        ]);
        let id = client.send_request("initialize", params)?;
        let response = client.wait(id, Duration::from_secs(15), cancel, Some(stop))?;
        let result = response
            .get("result")
            .ok_or("LSP initialize returned no result")?;
        client.capabilities = result
            .get("capabilities")
            .cloned()
            .ok_or("LSP initialize returned no capabilities")?;
        client.encoding = match client
            .capabilities
            .get("positionEncoding")
            .and_then(Value::as_str)
        {
            Some("utf-8") => Encoding::Utf8,
            Some("utf-32") => Encoding::Utf32,
            _ => Encoding::Utf16,
        };
        let sync = client.capabilities.get("textDocumentSync");
        client.sync_kind = sync
            .and_then(Value::as_usize)
            .or_else(|| sync?.get("change")?.as_usize())
            .unwrap_or(0);
        client.open_close = match sync {
            Some(Value::Number(_)) => client.sync_kind != 0,
            Some(Value::Object(_)) => matches!(
                sync.and_then(|v| v.get("openClose")),
                Some(Value::Bool(true))
            ),
            _ => false,
        };
        client.save = matches!(
            sync.and_then(|v| v.get("save")),
            Some(Value::Bool(true)) | Some(Value::Object(_))
        );
        client.save_include_text = matches!(
            sync.and_then(|v| v.get("save"))
                .and_then(|v| v.get("includeText")),
            Some(Value::Bool(true))
        );
        client.notify("initialized", Value::object([]))?;
        client.notify(
            "workspace/didChangeConfiguration",
            Value::object([("settings", profile.settings.clone())]),
        )?;
        Ok(client)
    }

    pub fn supports(&self, method: &str) -> bool {
        let key = match method {
            "textDocument/definition" => "definitionProvider",
            "textDocument/references" => "referencesProvider",
            "textDocument/diagnostic" => "diagnosticProvider",
            _ => return false,
        };
        matches!(
            self.capabilities.get(key),
            Some(Value::Bool(true)) | Some(Value::Object(_))
        )
    }

    pub fn query(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: &Cancellation,
        stop: &Cancellation,
    ) -> Result<Value, String> {
        if cancel.requested() {
            return Err("LSP request cancelled before dispatch".into());
        }
        if stop.requested() {
            return Err("LSP settings changed before dispatch".into());
        }
        self.partial_truncated = false;
        self.partial_bytes = 0;
        let mut params = params;
        let token = (method == "textDocument/references")
            .then(|| format!("jecode-partial-{}", self.next_id));
        if let Some(token) = &token {
            if let Value::Object(map) = &mut params {
                map.insert("partialResultToken".into(), Value::string(token));
            }
            self.partials.insert(token.clone(), Vec::new());
        }
        let id = self.send_request(method, params)?;
        let response = self.wait(id, timeout, cancel, Some(stop));
        let Some(token) = token else { return response };
        let partial = self.partials.remove(&token).unwrap_or_default();
        let mut response = response?;
        if !partial.is_empty()
            && let Value::Object(map) = &mut response
        {
            let mut all = partial;
            if let Some(Value::Array(final_values)) = map.get("result") {
                all.extend(final_values.iter().cloned());
            }
            map.insert("result".into(), Value::Array(all));
        }
        Ok(response)
    }

    pub fn shutdown(&mut self) {
        if let Ok(id) = self.send_request("shutdown", Value::Null) {
            let _ = self.wait(
                id,
                Duration::from_millis(500),
                &Cancellation::default(),
                None,
            );
        }
        let _ = self.notify("exit", Value::Null);
        self.wire.stop();
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.wire.send(Value::object([
            ("jsonrpc", Value::string("2.0")),
            ("id", Value::number(id)),
            ("method", Value::string(method)),
            ("params", params),
        ]))?;
        Ok(id)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.wire.send(Value::object([
            ("jsonrpc", Value::string("2.0")),
            ("method", Value::string(method)),
            ("params", params),
        ]))
    }

    fn wait(
        &mut self,
        id: u64,
        timeout: Duration,
        cancel: &Cancellation,
        stop: Option<&Cancellation>,
    ) -> Result<Value, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(message) = self.pending.remove(&id) {
                return response(message);
            }
            if cancel.requested() || stop.is_some_and(Cancellation::requested) {
                self.cancel(id);
                return Err(if stop.is_some_and(Cancellation::requested) {
                    "LSP settings changed"
                } else {
                    "LSP request cancelled"
                }
                .into());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.cancel(id);
                return Err("LSP request timed out".into());
            }
            let Some(message) = self.wire.recv(remaining.min(Duration::from_millis(20)))? else {
                continue;
            };
            if message.get("id").and_then(Value::as_usize) == Some(id as usize)
                && message.get("method").is_none()
            {
                return response(message);
            }
            self.handle(message)?;
        }
    }

    fn cancel(&mut self, id: u64) {
        self.cancelled.insert(id);
        let _ = self.notify(
            "$/cancelRequest",
            Value::object([("id", Value::number(id))]),
        );
    }

    pub fn drain(&mut self) -> Result<(), String> {
        while let Some(message) = self.wire.recv(Duration::from_millis(0))? {
            self.handle(message)?;
        }
        Ok(())
    }
}

fn client_capabilities() -> Value {
    Value::object([
        (
            "general",
            Value::object([(
                "positionEncodings",
                Value::Array(vec![Value::string("utf-16"), Value::string("utf-8")]),
            )]),
        ),
        (
            "workspace",
            Value::object([
                ("workspaceFolders", Value::Bool(true)),
                ("configuration", Value::Bool(true)),
                (
                    "didChangeWatchedFiles",
                    Value::object([("dynamicRegistration", Value::Bool(true))]),
                ),
            ]),
        ),
        (
            "textDocument",
            Value::object([
                ("definition", Value::object([])),
                ("references", Value::object([])),
                ("diagnostic", Value::object([])),
                (
                    "synchronization",
                    Value::object([("didSave", Value::Bool(true))]),
                ),
            ]),
        ),
    ])
}

fn response(message: Value) -> Result<Value, String> {
    if let Some(error) = message.get("error") {
        return Err(format!(
            "LSP server error: {}",
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
    }
    Ok(message)
}
