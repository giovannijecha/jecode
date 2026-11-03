use crate::json::Value;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq)]
pub struct CheckObservation {
    pub capabilities: Value,
    pub root: PathBuf,
    pub executable: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CheckFailure {
    pub detail: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub executable: String,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub languages: Vec<String>,
    pub extensions: BTreeMap<String, String>,
    pub root_markers: Vec<String>,
    pub root_override: Option<PathBuf>,
    pub initialization_options: Value,
    pub settings: Value,
    pub request_timeout_ms: u64,
    pub last_check: Option<CheckObservation>,
    pub last_check_failure: Option<CheckFailure>,
}

impl Profile {
    pub fn draft_custom() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let nonce = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            id: format!("lsp-{:x}-{:x}", now.as_nanos(), nonce),
            name: String::new(),
            enabled: true,
            executable: String::new(),
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            languages: Vec::new(),
            extensions: BTreeMap::new(),
            root_markers: Vec::new(),
            root_override: None,
            initialization_options: Value::Null,
            settings: Value::object([]),
            request_timeout_ms: 10_000,
            last_check: None,
            last_check_failure: None,
        }
    }

    pub fn draft_guided_rust() -> Self {
        let mut profile = Self::draft_custom();
        profile.name = "rust-analyzer".into();
        profile.executable = "rust-analyzer".into();
        profile.languages = vec!["rust".into()];
        profile.extensions.insert("rs".into(), "rust".into());
        profile.root_markers = vec!["Cargo.toml".into(), "rust-project.json".into()];
        profile
    }

    pub fn draft_guided_cpp() -> Self {
        let mut profile = Self::draft_custom();
        profile.name = "clangd".into();
        profile.executable = "clangd".into();
        profile.languages = vec!["c".into(), "cpp".into()];
        for (extension, language) in [
            ("c", "c"),
            ("h", "c"),
            ("cc", "cpp"),
            ("cpp", "cpp"),
            ("hpp", "cpp"),
        ] {
            profile.extensions.insert(extension.into(), language.into());
        }
        profile.root_markers = vec![
            "compile_commands.json".into(),
            "compile_flags.txt".into(),
            ".clangd".into(),
        ];
        profile
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(
                "LSP server ID must contain letters, digits, hyphens or underscores".into(),
            );
        }
        if self.name.trim().is_empty() || self.executable.trim().is_empty() {
            return Err("LSP server name and executable are required".into());
        }
        if self.extensions.is_empty() || self.languages.is_empty() {
            return Err("LSP server languages and extensions are required".into());
        }
        if self.extensions.iter().any(|(extension, language)| {
            extension.is_empty() || extension.contains('.') || !self.languages.contains(language)
        }) {
            return Err(
                "LSP file associations need an extension and configured language ID".into(),
            );
        }
        if self.root_markers.iter().any(|marker| {
            marker.is_empty()
                || !Path::new(marker)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
        }) {
            return Err("LSP root markers must be relative paths without parent traversal".into());
        }
        if !(100..=120_000).contains(&self.request_timeout_ms) {
            return Err("LSP request timeout must be 100–120000 ms".into());
        }
        if self
            .environment
            .keys()
            .any(|key| key.is_empty() || key.contains('=') || key.contains('\0'))
        {
            return Err("Invalid LSP environment variable name".into());
        }
        Ok(())
    }

    pub fn same_analysis_settings(&self, other: &Self) -> bool {
        self.executable == other.executable
            && self.arguments == other.arguments
            && self.environment == other.environment
            && self.languages == other.languages
            && self.extensions == other.extensions
            && self.root_markers == other.root_markers
            && self.root_override == other.root_override
            && self.initialization_options == other.initialization_options
            && self.settings == other.settings
            && self.request_timeout_ms == other.request_timeout_ms
    }

    pub(crate) fn language_for(&self, path: &std::path::Path) -> Option<&str> {
        let extension = path.extension()?.to_str()?;
        self.extensions.get(extension).map(String::as_str)
    }

    pub(crate) fn matches(&self, path: &std::path::Path) -> bool {
        self.language_for(path).is_some()
    }

    pub fn analysis_fingerprint(&self) -> String {
        let mut profile = self.clone();
        profile.id.clear();
        profile.name.clear();
        profile.enabled = false;
        profile.last_check = None;
        profile.last_check_failure = None;
        let value = super::store::encode(&profile).encode();
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in value.bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
    }

    pub fn instance_fingerprint(&self) -> String {
        let text = format!("{}:{}", self.name, self.analysis_fingerprint());
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in text.bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
    }
}
