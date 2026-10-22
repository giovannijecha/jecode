//! Checkpoint scope and limits. The rules are Jecode's own: Git ignore files
//! and the Git index are never consulted.
use super::hash;
use crate::json::{self, Value};
use std::path::Path;

pub const DEFAULT_MAX_FILE_BYTES: u64 = 10 * MIB;
pub const DEFAULT_BUDGET_BYTES: u64 = 1024 * MIB;
const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const GIB: u64 = 1024 * MIB;
const MIN_BUDGET: u64 = MIB;
const MAX_LIMIT: u64 = DEFAULT_BUDGET_BYTES;
const MAX_FILE_LIMIT: u64 = DEFAULT_MAX_FILE_BYTES;
const MAX_RULES: usize = 256;

/// VCS metadata, Jecode output and generated build or cache directories.
pub const DEFAULT_EXCLUDE: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".jj",
    ".bzr",
    "_darcs",
    ".jecode-output",
    ".jecode-*.tmp",
    "target",
    "node_modules",
    "__pycache__",
    ".venv",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    ".gradle",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    ".svelte-kit",
    ".terraform",
    ".cache",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    pub max_file_bytes: u64,
    pub budget_bytes: u64,
    pub exclude: Vec<String>,
    pub include: Vec<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            budget_bytes: DEFAULT_BUDGET_BYTES,
            exclude: DEFAULT_EXCLUDE
                .iter()
                .map(|rule| rule.to_string())
                .collect(),
            include: Vec::new(),
        }
    }
}

impl Policy {
    /// Reads `policy.json`, the file users edit; `None` when it does not exist.
    pub fn read(directory: &Path) -> Result<Option<Self>, String> {
        let path = directory.join("policy.json");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Could not read checkpoint policy: {error}")),
        };
        Self::parse(&text)
            .map(Some)
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let value = json::parse(text)?;
        let limit = |key: &str, default: u64| {
            value.get(key).map_or(Ok(default), |value| {
                value
                    .as_usize()
                    .map(|value| value as u64)
                    .ok_or(format!("{key} must be a byte count"))
            })
        };
        let rules = |key: &str, default: &[&str]| -> Result<Vec<String>, String> {
            let Some(value) = value.get(key) else {
                return Ok(default.iter().map(|rule| rule.to_string()).collect());
            };
            value
                .as_array()
                .ok_or(format!("{key} must be a list of patterns"))?
                .iter()
                .map(|rule| {
                    rule.as_str()
                        .map(str::to_string)
                        .ok_or(format!("{key} must contain strings"))
                })
                .collect()
        };
        let policy = Self {
            // Old policy files may name larger values. Bound them while
            // loading so their existing journal remains readable. New policy
            // values supplied through `save` still undergo strict validation.
            max_file_bytes: limit("max_file_bytes", DEFAULT_MAX_FILE_BYTES)?.min(MAX_FILE_LIMIT),
            budget_bytes: limit("budget_bytes", DEFAULT_BUDGET_BYTES)?.min(MAX_LIMIT),
            exclude: rules("exclude", DEFAULT_EXCLUDE)?,
            include: rules("include", &[])?,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn encode(&self) -> String {
        let list = |rules: &[String]| Value::Array(rules.iter().map(Value::string).collect());
        Value::object([
            ("version", Value::number(1)),
            ("max_file_bytes", Value::number(self.max_file_bytes)),
            ("budget_bytes", Value::number(self.budget_bytes)),
            ("exclude", list(&self.exclude)),
            ("include", list(&self.include)),
        ])
        .pretty()
    }

    #[cfg(test)]
    pub fn save(&self, directory: &Path) -> Result<(), String> {
        let _lock = super::locks::Exclusive::acquire(&directory.join("store.lock"))
            .map_err(|error| format!("Could not lock checkpoint archive: {error}"))?;
        self.save_locked(directory)
    }

    /// Save while the caller holds the archive store lock.
    pub fn save_locked(&self, directory: &Path) -> Result<(), String> {
        self.validate()?;
        let text = self.encode();
        super::quota::admit(directory, text.len() as u64, false, super::quota::LIMIT)?;
        super::files::atomic_write(&directory.join("policy.json"), text.as_bytes())
            .map_err(|error| format!("Could not save checkpoint policy: {error}"))
    }

    pub fn id(&self) -> u64 {
        hash::digest(self.encode().as_bytes()).0[0]
    }

    /// Describes what differs from `previous`, such as `+exclude *.log`.
    pub fn changes(&self, previous: &Self) -> Vec<String> {
        let mut changes = Vec::new();
        if self.budget_bytes != previous.budget_bytes {
            changes.push(format!("budget {}", size(self.budget_bytes)));
        }
        if self.max_file_bytes != previous.max_file_bytes {
            changes.push(format!("maximum file {}", size(self.max_file_bytes)));
        }
        for (name, now, before) in [
            ("exclude", &self.exclude, &previous.exclude),
            ("include", &self.include, &previous.include),
        ] {
            let added = now.iter().filter(|rule| !before.contains(rule));
            changes.extend(added.map(|rule| format!("+{name} {rule}")));
            let removed = before.iter().filter(|rule| !now.contains(rule));
            changes.extend(removed.map(|rule| format!("-{name} {rule}")));
        }
        changes
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_BUDGET..=MAX_LIMIT).contains(&self.budget_bytes) {
            return Err(format!(
                "The archive budget must be between {} and {}",
                size(MIN_BUDGET),
                size(MAX_LIMIT)
            ));
        }
        if self.max_file_bytes == 0
            || self.max_file_bytes > MAX_FILE_LIMIT
            || self.max_file_bytes > self.budget_bytes
        {
            return Err(format!(
                "The maximum file size must be between 1 B and {}, and not above the archive budget",
                size(MAX_FILE_LIMIT)
            ));
        }
        for rule in self.exclude.iter().chain(&self.include) {
            validate_rule(rule)?;
        }
        if self.exclude.len() + self.include.len() > MAX_RULES {
            return Err(format!("At most {MAX_RULES} scope rules are supported"));
        }
        Ok(())
    }

    /// Whether a workspace-relative path is in scope. Include rules override
    /// exclusion of the path they match; an excluded directory is not visited.
    pub fn eligible(&self, relative: &str, directory: bool) -> bool {
        let matches =
            |rules: &[String]| rules.iter().any(|rule| matches(rule, relative, directory));
        !matches(&self.exclude) || matches(&self.include)
    }
}

fn validate_rule(rule: &str) -> Result<(), String> {
    let body = rule.strip_suffix('/').unwrap_or(rule);
    if body.is_empty()
        || rule.len() > 256
        || body.starts_with('/')
        || body.contains('\\')
        || body
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!(
            "Invalid scope rule {rule:?}: use a name like target, a relative path like docs/build or a glob like *.log"
        ));
    }
    Ok(())
}

/// A rule without `/` matches any path component name; a rule with `/`
/// matches the whole relative path. A trailing `/` limits it to directories.
fn matches(rule: &str, relative: &str, directory: bool) -> bool {
    let (rule, directory_only) = match rule.strip_suffix('/') {
        Some(rule) => (rule, true),
        None => (rule, false),
    };
    if directory_only && !directory {
        return false;
    }
    if rule.contains('/') {
        glob(rule.as_bytes(), relative.as_bytes(), true)
    } else {
        let name = relative.rsplit('/').next().unwrap_or(relative);
        glob(rule.as_bytes(), name.as_bytes(), false)
    }
}

enum Token {
    Byte(u8),
    One,
    Star,
    Deep,
    Directories,
}

/// `*` and `?` stay inside one component; `**` spans components and `**/`
/// matches zero or more whole directories. Linear in pattern times text.
fn glob(pattern: &[u8], text: &[u8], paths: bool) -> bool {
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < pattern.len() {
        let token = match pattern[index] {
            b'*' if paths && pattern[index..].starts_with(b"**/") => {
                index += 2;
                Token::Directories
            }
            b'*' if paths && pattern[index..].starts_with(b"**") => {
                index += 1;
                Token::Deep
            }
            b'*' => Token::Star,
            b'?' => Token::One,
            byte => Token::Byte(byte),
        };
        tokens.push(token);
        index += 1;
    }
    let size = text.len();
    let mut next = vec![false; size + 1];
    next[size] = true;
    for token in tokens.iter().rev() {
        let mut current = vec![false; size + 1];
        let mut directories = false;
        for at in (0..=size).rev() {
            let here = text.get(at).copied();
            current[at] = match token {
                Token::Byte(byte) => here == Some(*byte) && next[at + 1],
                Token::One => here.is_some_and(|byte| byte != b'/') && next[at + 1],
                Token::Star => next[at] || here.is_some_and(|byte| byte != b'/') && current[at + 1],
                Token::Deep => next[at] || here.is_some() && current[at + 1],
                Token::Directories => {
                    directories |= here == Some(b'/') && next[at + 1];
                    next[at] || directories
                }
            };
        }
        next = current;
    }
    next[0]
}

pub fn parse_size(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let split = text
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let multiplier = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => KIB,
        "m" | "mb" | "mib" => MIB,
        "g" | "gb" | "gib" => GIB,
        "t" | "tb" | "tib" => 1024 * GIB,
        _ => {
            return Err(format!(
                "Unknown size unit in {text:?}; use B, KiB, MiB or GiB"
            ));
        }
    };
    let value: f64 = number
        .parse()
        .map_err(|_| format!("Invalid size {text:?}"))?;
    let bytes = value * multiplier as f64;
    if !bytes.is_finite() || bytes < 0.0 || bytes > MAX_LIMIT as f64 {
        return Err(format!("Invalid size {text:?}"));
    }
    Ok(bytes.round() as u64)
}

pub fn size(bytes: u64) -> String {
    for (unit, scale) in [
        ("TiB", 1024 * GIB),
        ("GiB", GIB),
        ("MiB", MIB),
        ("KiB", KIB),
    ] {
        if bytes >= scale {
            let value = bytes as f64 / scale as f64;
            return if bytes.is_multiple_of(scale) {
                format!("{} {unit}", bytes / scale)
            } else {
                format!("{value:.1} {unit}")
            };
        }
    }
    format!("{bytes} B")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scope_keeps_ignored_project_files_and_skips_generated_trees() {
        let policy = Policy::default();
        for path in ["JECODE.md", ".env", "src/main.rs", "dist", ".gitignore"] {
            assert!(policy.eligible(path, false), "{path}");
        }
        for (path, directory) in [
            (".git", true),
            ("crates/a/target", true),
            ("web/node_modules", true),
            ("src/.jecode-1-2.tmp", false),
        ] {
            assert!(!policy.eligible(path, directory), "{path}");
        }
    }

    #[test]
    fn rules_match_names_paths_globs_and_directories() {
        assert!(matches("*.log", "a/b/c.log", false));
        assert!(!matches("*.log", "a/b/c.log.txt", false));
        assert!(matches("docs/build", "docs/build", true));
        assert!(!matches("docs/build", "x/docs/build", true));
        assert!(matches("**/fixtures", "a/b/fixtures", true));
        assert!(matches("src/**/*.rs", "src/a/b.rs", false));
        assert!(matches("src/**/*.rs", "src/b.rs", false));
        assert!(!matches("src/*.rs", "src/a/b.rs", false));
        assert!(matches("cache/", "a/cache", true));
        assert!(!matches("cache/", "a/cache", false));
        let mut policy = Policy::default();
        policy.include.push("target".into());
        assert!(policy.eligible("target", true));
    }

    #[test]
    fn limits_are_validated_persisted_and_identified() {
        let directory = crate::test_support::Directory::new();
        let mut policy = Policy::default();
        assert_eq!(policy.max_file_bytes, 10 * 1024 * 1024);
        assert_eq!(policy.budget_bytes, 1024 * 1024 * 1024);
        policy.max_file_bytes = parse_size("2 MiB").unwrap();
        policy.exclude.push("*.bin".into());
        policy.save(directory.path()).unwrap();
        let loaded = Policy::read(directory.path()).unwrap().unwrap();
        assert_eq!(loaded, policy);
        assert_ne!(loaded.id(), Policy::default().id());
        assert_eq!(
            loaded.changes(&Policy::default()),
            ["maximum file 2 MiB", "+exclude *.bin"]
        );
        assert_eq!(Policy::read(&directory.path().join("absent")), Ok(None));
        policy.budget_bytes = 10;
        assert!(policy.validate().is_err());
        assert!(Policy::parse(r#"{"exclude":["../x"]}"#).is_err());
        assert!(Policy::parse(r#"{"max_file_bytes":0}"#).is_err());
        let legacy =
            Policy::parse(r#"{"max_file_bytes":4294967296,"budget_bytes":17179869184}"#).unwrap();
        assert_eq!(legacy.max_file_bytes, DEFAULT_MAX_FILE_BYTES);
        assert_eq!(legacy.budget_bytes, DEFAULT_BUDGET_BYTES);
        policy.budget_bytes = DEFAULT_BUDGET_BYTES;
        policy.max_file_bytes = DEFAULT_MAX_FILE_BYTES + 1;
        assert!(policy.save(directory.path()).is_err());
        policy.max_file_bytes = DEFAULT_MAX_FILE_BYTES;
        policy.budget_bytes = DEFAULT_BUDGET_BYTES + 1;
        assert!(policy.save(directory.path()).is_err());
        assert_eq!(parse_size("1.5k").unwrap(), 1536);
        assert_eq!(size(10 * 1024 * 1024), "10 MiB");
        assert!(parse_size("3 parsecs").is_err());
    }
}
