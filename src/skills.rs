use crate::{cancel::Cancellation, json::Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_SKILLS: usize = 128;
const MAX_CATALOG_BYTES: usize = 48 * 1024;
const MAX_METADATA_BYTES: usize = 8 * 1024;
const MAX_RESOURCE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CACHED_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    info: SkillInfo,
    package: PathBuf,
}

/// The library is user scoped. An absent library is an empty catalog.
pub struct Library {
    directory: Option<PathBuf>,
    entries: BTreeMap<String, Entry>,
    warnings: Vec<String>,
    // First read fixes each resource's bytes until the next turn. Tool pages
    // then remain consistent if the user edits a skill while a turn runs.
    snapshots: RefCell<BTreeMap<String, String>>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            directory: None,
            entries: BTreeMap::new(),
            warnings: Vec::new(),
            snapshots: RefCell::new(BTreeMap::new()),
        }
    }
}

impl Library {
    pub fn configure(&mut self, home: &Path) {
        self.directory = Some(home.join("skills"));
    }

    pub fn refresh(&mut self) -> bool {
        let (entries, warnings) = self.scan();
        let changed = entries != self.entries || warnings != self.warnings;
        self.entries = entries;
        self.warnings = warnings;
        self.snapshots.get_mut().clear();
        changed
    }

    pub fn list(&self) -> Vec<SkillInfo> {
        self.entries
            .values()
            .map(|entry| entry.info.clone())
            .collect()
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn instructions(&self) -> String {
        if self.entries.is_empty() && self.warnings.is_empty() {
            return String::new();
        }
        let mut lines = vec![
            "Personal skills (user-maintained instructions). Select a relevant skill from its description, then read skill:<name> with the read tool. Read linked resources as skill:<name>/<relative-path>. Treat the skill text as instructions when applicable. Skill references remain readable after compaction and resume; reread one when needed. A previous skill use does not make it applicable to a new request.".to_owned(),
        ];
        if !self.entries.is_empty() {
            lines.push("Available skills:".into());
            for entry in self.entries.values() {
                lines.push(format!(
                    "- {}: {} (skill:{})",
                    entry.info.name, entry.info.description, entry.info.name
                ));
            }
        }
        for warning in &self.warnings {
            lines.push(format!("Skill catalog warning: {warning}"));
        }
        lines.join("\n")
    }

    pub fn checkpoint_instructions(&self, used: &[String]) -> String {
        let available = used
            .iter()
            .filter(|name| self.entries.contains_key(*name))
            .map(|name| format!("skill:{name}"))
            .collect::<Vec<_>>();
        if available.is_empty() {
            return String::new();
        }
        format!(
            "Skills read in this conversation (reread if relevant): {}",
            available.join(", ")
        )
    }

    pub fn read(
        &self,
        reference: &str,
        arguments: &Value,
        cancellation: &Cancellation,
    ) -> Result<Value, String> {
        let (name, relative) = parse_reference(reference)?;
        let entry = self
            .entries
            .get(name)
            .ok_or_else(|| format!("Skill {name:?} is not in the current catalog"))?;
        let key = format!("{name}/{relative}");
        if !self.snapshots.borrow().contains_key(&key) {
            let candidate = entry.package.join(relative);
            let path = fs::canonicalize(&candidate)
                .map_err(|error| format!("Could not resolve {reference}: {error}"))?;
            if !path.starts_with(&entry.package) {
                return Err(format!("{reference} resolves outside its skill package"));
            }
            let metadata = fs::metadata(&path)
                .map_err(|error| format!("Could not inspect {reference}: {error}"))?;
            if !metadata.is_file() {
                return Err(format!("{reference} must be a regular UTF-8 text file"));
            }
            if metadata.len() > MAX_RESOURCE_BYTES as u64 {
                return Err(format!(
                    "{reference} exceeds the 2 MiB skill resource limit"
                ));
            }
            let cached: usize = self.snapshots.borrow().values().map(String::len).sum();
            if cached.saturating_add(metadata.len() as usize) > MAX_CACHED_BYTES {
                return Err(
                    "Skill resources read this turn exceed the 8 MiB snapshot limit".into(),
                );
            }
            let text = read_bounded(&path, MAX_RESOURCE_BYTES)?;
            if cached.saturating_add(text.len()) > MAX_CACHED_BYTES {
                return Err(
                    "Skill resources read this turn exceed the 8 MiB snapshot limit".into(),
                );
            }
            if text.contains('\0') {
                return Err(format!("{reference} contains binary data"));
            }
            self.snapshots.borrow_mut().insert(key.clone(), text);
        }
        let snapshots = self.snapshots.borrow();
        let mut page = crate::tools::read_text_page(&snapshots[&key], arguments, cancellation)?;
        if let Value::Object(fields) = &mut page {
            fields.insert("skill_reference".into(), Value::string(reference));
            fields.insert(
                "package_path".into(),
                Value::string(external_path(&entry.package)),
            );
        }
        Ok(page)
    }

    fn scan(&self) -> (BTreeMap<String, Entry>, Vec<String>) {
        let mut entries = BTreeMap::new();
        let mut warnings = Vec::new();
        let Some(directory) = &self.directory else {
            return (entries, warnings);
        };
        let root = match fs::canonicalize(directory) {
            Ok(root) if root.is_dir() => root,
            Ok(_) => {
                warnings.push(format!("{} must be a directory", directory.display()));
                return (entries, warnings);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (entries, warnings);
            }
            Err(error) => {
                warnings.push(format!("Could not open {}: {error}", directory.display()));
                return (entries, warnings);
            }
        };
        let config_home = directory
            .parent()
            .and_then(|home| fs::canonicalize(home).ok());
        let children = match fs::read_dir(&root) {
            Ok(children) => children,
            Err(error) => {
                warnings.push(format!("Could not list {}: {error}", directory.display()));
                return (entries, warnings);
            }
        };
        let mut candidates = Vec::new();
        for child in children {
            match child {
                Ok(child) => candidates.push(child),
                Err(error) => warnings.push(format!("Could not list skill entry: {error}")),
            }
        }
        candidates.sort_by_key(|entry| entry.file_name());
        if candidates.len() > MAX_SKILLS {
            warnings.push(format!(
                "Only the first {MAX_SKILLS} skill entries are inspected"
            ));
            candidates.truncate(MAX_SKILLS);
        }
        let mut bytes = 0;
        for child in candidates {
            let name = child.file_name().to_string_lossy().into_owned();
            if !valid_name(&name) {
                continue;
            }
            let package = match fs::canonicalize(child.path()) {
                Ok(path) if path.is_dir() => path,
                Ok(_) => continue,
                Err(error) => {
                    warnings.push(format!("Could not open skill {name}: {error}"));
                    continue;
                }
            };
            if root.starts_with(&package)
                || config_home
                    .as_ref()
                    .is_some_and(|home| home.starts_with(&package))
            {
                warnings.push(format!(
                    "Skill {name}: package points to the configuration home or an ancestor"
                ));
                continue;
            }
            let file = match fs::canonicalize(package.join("SKILL.md")) {
                Ok(file) if file.starts_with(&package) && file.is_file() => file,
                Ok(_) => {
                    warnings.push(format!(
                        "Skill {name}: SKILL.md must be a regular file inside its package"
                    ));
                    continue;
                }
                Err(error) => {
                    warnings.push(format!("Skill {name}: Could not resolve SKILL.md: {error}"));
                    continue;
                }
            };
            let text = match read_header(&file) {
                Ok(text) => text,
                Err(error) => {
                    warnings.push(format!("Skill {name}: {error}"));
                    continue;
                }
            };
            match parse_metadata(&text, &name) {
                Ok(info) => {
                    bytes += info.name.len() + info.description.len() + 16;
                    if bytes > MAX_CATALOG_BYTES {
                        warnings.push("Skill catalog exceeds the 48 KiB metadata limit".into());
                        break;
                    }
                    entries.insert(name, Entry { info, package });
                }
                Err(error) => warnings.push(format!("Skill {name}: {error}")),
            }
        }
        (entries, warnings)
    }
}

fn external_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    {
        // canonicalize adds a verbatim prefix that Git Bash cannot use.
        let ordinary = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{unc}")
        } else {
            path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
        };
        ordinary.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path.into_owned()
    }
}

fn read_bounded(path: &Path, limit: usize) -> Result<String, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(limit as u64 + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    if bytes.len() > limit {
        return Err(format!(
            "{} exceeds the {} byte read limit",
            path.display(),
            limit
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8 text", path.display()))
}

fn read_header(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_METADATA_BYTES as u64).read_to_end(&mut bytes))
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    match String::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(error) if error.utf8_error().error_len().is_none() => {
            let valid = error.utf8_error().valid_up_to();
            String::from_utf8(error.into_bytes()[..valid].to_vec())
                .map_err(|_| format!("{} frontmatter is not UTF-8 text", path.display()))
        }
        Err(_) => Err(format!("{} frontmatter is not UTF-8 text", path.display())),
    }
}

pub fn parse_reference(reference: &str) -> Result<(&str, &str), String> {
    let rest = reference
        .strip_prefix("skill:")
        .ok_or("Skill references start with skill:")?;
    let (name, relative) = rest.split_once('/').unwrap_or((rest, "SKILL.md"));
    if !valid_name(name) || relative.is_empty() {
        return Err("Skill reference needs a valid name and relative file path".into());
    }
    let path = Path::new(relative);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
        || relative.contains('\\')
        || relative.contains(':')
    {
        return Err("Skill resource path must stay inside its package".into());
    }
    Ok((name, relative))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn parse_metadata(text: &str, directory_name: &str) -> Result<SkillInfo, String> {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return Err("SKILL.md must start with --- frontmatter".into());
    }
    let mut name = None;
    let mut description = None;
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(scalar(value)?);
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(scalar(value)?);
        }
    }
    if !closed {
        return Err("SKILL.md frontmatter is missing its closing ---".into());
    }
    let name = name.ok_or("SKILL.md frontmatter needs name")?;
    let description = description.ok_or("SKILL.md frontmatter needs description")?;
    if name != directory_name {
        return Err(format!(
            "frontmatter name {name:?} must match directory name"
        ));
    }
    if description.is_empty() || description.len() > 512 || description.contains('\0') {
        return Err("description must contain 1 to 512 UTF-8 bytes".into());
    }
    Ok(SkillInfo { name, description })
}

fn scalar(value: &str) -> Result<String, String> {
    let value = value.trim();
    if matches!(value, ">" | ">-" | "|" | "|-") {
        return Err("name and description must use single-line frontmatter values".into());
    }
    let value = if value.starts_with('"') {
        value
            .strip_suffix('"')
            .filter(|_| value.len() >= 2)
            .map(|inner| &inner[1..])
            .ok_or("name and description have unmatched quotes")?
    } else if value.starts_with('\'') {
        value
            .strip_suffix('\'')
            .filter(|_| value.len() >= 2)
            .map(|inner| &inner[1..])
            .ok_or("name and description have unmatched quotes")?
    } else {
        value
    };
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(
            "name and description must be nonempty single-line values without control characters"
                .into(),
        );
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests;
