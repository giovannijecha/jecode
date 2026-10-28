use super::{Mode, Workspace, files, git};
use crate::disk::hash;
use crate::json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

struct Entry {
    path: String,
    baseline: files::Fingerprint,
    child: files::Fingerprint,
    current: files::Fingerprint,
}

impl Entry {
    fn conflict(&self) -> bool {
        self.current != self.baseline && self.current != self.child
    }
    fn status(&self) -> &'static str {
        if !self.baseline.exists() {
            "added"
        } else if !self.child.exists() {
            "deleted"
        } else if self.baseline.label() != self.child.label() {
            "type_changed"
        } else {
            "modified"
        }
    }
    fn value(&self) -> Value {
        Value::object([
            ("path", Value::string(&self.path)),
            ("status", Value::string(self.status())),
            ("conflict", Value::Bool(self.conflict())),
            ("baseline", self.baseline.value()),
            ("child", self.child.value()),
            ("current", self.current.value()),
        ])
    }
}

pub(super) struct Review {
    entries: Vec<Entry>,
    token: String,
}

impl Review {
    pub(super) fn value(self, workspace: &Workspace) -> Result<Value, String> {
        let mut changed = Vec::new();
        for entry in &self.entries {
            let mut value = entry.value();
            if let Value::Object(fields) = &mut value {
                fields.insert("evidence".into(), evidence(workspace, entry)?);
            }
            changed.push(value);
        }
        Ok(Value::object([
            ("changed", Value::Array(changed)),
            ("review_token", Value::string(self.token)),
            (
                "scope",
                Value::string("Git tracked and non-ignored untracked paths"),
            ),
        ]))
    }
}

pub(super) fn inspect(workspace: &Workspace) -> Result<Review, String> {
    if workspace.mode == Mode::Shared {
        return Ok(Review {
            entries: Vec::new(),
            token: "shared".into(),
        });
    }
    workspace.verify_owner()?;
    let mut names: BTreeSet<_> = workspace.baseline.keys().cloned().collect();
    names.extend(git::inventory(&workspace.root)?);
    let mut entries = Vec::new();
    for name in names {
        files::validate_name(&name)?;
        let baseline = workspace
            .baseline
            .get(&name)
            .cloned()
            .unwrap_or(files::Fingerprint::Absent);
        let child = files::observe(&workspace.root, &name)?;
        if child == baseline {
            continue;
        }
        let current = files::observe(&workspace.project, &name)?;
        entries.push(Entry {
            path: name,
            baseline,
            child,
            current,
        });
    }
    let encoded = Value::Array(entries.iter().map(Entry::value).collect()).encode();
    let digest = hash::digest(encoded.as_bytes());
    let token = format!("{:016x}{:016x}", digest.0[0], digest.0[1]);
    Ok(Review { entries, token })
}

pub(super) fn integrate(
    workspace: &Workspace,
    paths: &[String],
    token: &str,
) -> Result<Value, String> {
    if workspace.mode == Mode::Shared {
        return Err(
            "Shared work is already in the project; there is no separate change to integrate"
                .into(),
        );
    }
    let review = inspect(workspace)?;
    if review.token != token {
        return Err("Workspace or project changed since review; inspect again".into());
    }
    let selected: BTreeSet<_> = paths.iter().cloned().collect();
    if selected.len() != paths.len() {
        return Err("Duplicate integration path".into());
    }
    for path in &selected {
        files::validate_name(path)?;
        if !review.entries.iter().any(|entry| &entry.path == path) {
            return Err(format!("{path} is not a changed path in this review"));
        }
    }
    let mut applied = Vec::new();
    let mut already = Vec::new();
    let mut conflicts = Vec::new();
    let mut failed = Vec::new();
    for entry in review
        .entries
        .iter()
        .filter(|entry| selected.contains(&entry.path))
    {
        files::verify_content(
            &workspace.storage.join("base"),
            &entry.path,
            &entry.baseline,
        )?;
        let current = files::observe(&workspace.project, &entry.path)?;
        let child = files::observe(&workspace.root, &entry.path)?;
        if current != entry.current || child != entry.child {
            conflicts.push(issue(&entry.path, "changed_since_review"));
            break;
        }
        if current == child {
            already.push(Value::string(&entry.path));
            continue;
        }
        if entry.conflict() {
            conflicts.push(issue(&entry.path, "project_changed_since_baseline"));
            continue;
        }
        let result = install(workspace, entry);
        if let Err(error) = result {
            failed.push(Value::object([
                ("path", Value::string(&entry.path)),
                ("reason", Value::string(error)),
            ]));
            break;
        }
        applied.push(Value::string(&entry.path));
    }
    Ok(Value::object([
        ("applied", Value::Array(applied)),
        ("already_current", Value::Array(already)),
        ("conflicts", Value::Array(conflicts)),
        ("failed", Value::Array(failed)),
        ("review_token", Value::string(review.token)),
    ]))
}

fn issue(path: &str, reason: &str) -> Value {
    Value::object([
        ("path", Value::string(path)),
        ("reason", Value::string(reason)),
    ])
}

fn install(workspace: &Workspace, entry: &Entry) -> Result<(), String> {
    let name = &entry.path;
    match &entry.child {
        files::Fingerprint::Directory | files::Fingerprint::Link { .. } => {
            Err("Directory and link integration requires coordinator handling".into())
        }
        files::Fingerprint::Absent => {
            if !matches!(
                entry.current,
                files::Fingerprint::File { .. } | files::Fingerprint::Absent
            ) {
                return Err("Deletion of a non-file requires coordinator handling".into());
            }
            let destination = files::path(&workspace.project, name)?;
            if files::observe(&workspace.project, name)? != entry.current {
                return Err("Project changed before deletion".into());
            }
            crate::checkpoints::files::remove(&destination).map_err(|error| error.to_string())?;
            Ok(())
        }
        files::Fingerprint::File { mode, .. } => {
            if !matches!(
                entry.current,
                files::Fingerprint::File { .. } | files::Fingerprint::Absent
            ) {
                return Err("Replacement of a non-file requires coordinator handling".into());
            }
            let source = files::path(&workspace.root, name)?;
            let destination = files::path(&workspace.project, name)?;
            create_parents(&workspace.project, name)?;
            let input = fs::File::open(&source).map_err(|error| error.to_string())?;
            #[cfg(unix)]
            let mode = crate::checkpoints::files::Mode(*mode);
            #[cfg(not(unix))]
            let mode = crate::checkpoints::files::Mode(if *mode != 0 {
                crate::checkpoints::files::Mode::READONLY
            } else {
                0
            });
            let staged = crate::checkpoints::files::stage_reader(&destination, input, mode)
                .map_err(|error| format!("Could not stage {name}: {error}"))?;
            if files::observe(&workspace.root, name)? != entry.child
                || files::observe(&workspace.project, name)? != entry.current
            {
                return Err("File changed before installation".into());
            }
            staged
                .install_deferred()
                .map_err(|error| format!("Could not install {name}: {error}"))
        }
    }
}

fn create_parents(root: &Path, name: &str) -> Result<(), String> {
    let mut path = root.to_path_buf();
    for part in name
        .split('/')
        .take(name.split('/').count().saturating_sub(1))
    {
        path.push(part);
        match crate::disk::metadata_if_present(&path).map_err(|error| error.to_string())? {
            Some(metadata) if crate::disk::ordinary_directory(&metadata) => {}
            Some(_) => return Err(format!("{name} has a linked or non-directory parent")),
            None => fs::create_dir(&path).map_err(|error| error.to_string())?,
        }
    }
    Ok(())
}

fn evidence(workspace: &Workspace, entry: &Entry) -> Result<Value, String> {
    let regular = |state: &files::Fingerprint| {
        matches!(
            state,
            files::Fingerprint::File { .. } | files::Fingerprint::Absent
        )
    };
    if !regular(&entry.baseline) || !regular(&entry.child) {
        return Ok(Value::object([
            ("kind", Value::string("metadata")),
            (
                "detail",
                Value::string(
                    "Inspect link or directory state; integration requires coordinator handling",
                ),
            ),
        ]));
    }
    let max = crate::file_change::MAX_DIFF_BYTES as u64;
    if [&entry.baseline, &entry.child]
        .iter()
        .any(|state| matches!(state, files::Fingerprint::File { size, .. } if *size > max))
    {
        return Ok(Value::object([
            ("kind", Value::string("large_file")),
            (
                "detail",
                Value::string("Use diff with path and byte offset to inspect content"),
            ),
        ]));
    }
    let before = read_small(
        &workspace.storage.join("base"),
        &entry.path,
        &entry.baseline,
    )?;
    let after = read_small(&workspace.root, &entry.path, &entry.child)?;
    let (before, after) = match (before, after) {
        (Some(before), Some(after)) => (before, after),
        _ => {
            return Ok(Value::object([
                ("kind", Value::string("binary")),
                (
                    "detail",
                    Value::string(
                        "Binary content or invalid UTF-8; fingerprints give exact sizes and content identity",
                    ),
                ),
            ]));
        }
    };
    if let Some(change) = crate::file_change::Change::between(
        entry.baseline.exists().then_some(before.as_str()),
        &after,
    ) {
        let value = change.value();
        if value.encode().len() <= 16_384 {
            return Ok(Value::object([
                ("kind", Value::string("text_diff")),
                ("diff", value),
            ]));
        }
    }
    Ok(Value::object([
        ("kind", Value::string("text_preview")),
        (
            "before",
            Value::string(&before[..before.floor_char_boundary(2048)]),
        ),
        (
            "after",
            Value::string(&after[..after.floor_char_boundary(2048)]),
        ),
        (
            "detail",
            Value::string(
                "Diff is larger than the inline limit; use diff with path and byte offset",
            ),
        ),
    ]))
}

fn read_small(
    root: &Path,
    name: &str,
    state: &files::Fingerprint,
) -> Result<Option<String>, String> {
    if matches!(state, files::Fingerprint::Absent) {
        return Ok(Some(String::new()));
    }
    files::verify_content(root, name, state)?;
    let bytes = fs::read(files::path(root, name)?).map_err(|error| error.to_string())?;
    files::verify_content(root, name, state)?;
    Ok(String::from_utf8(bytes)
        .ok()
        .filter(|text| !text.contains('\0')))
}

pub(super) fn inspect_file(
    workspace: &Workspace,
    path: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    if workspace.mode != Mode::Isolated {
        return Err("Shared work has no separate baseline".into());
    }
    if limit == 0 || limit > 65_536 {
        return Err("limit must be 1-65536 bytes".into());
    }
    files::validate_name(path)?;
    let review = inspect(workspace)?;
    let entry = review
        .entries
        .iter()
        .find(|entry| entry.path == path)
        .ok_or("Path is not changed in this workspace")?;
    let before = page(
        &workspace.storage.join("base"),
        path,
        &entry.baseline,
        offset,
        limit,
    )?;
    let after = page(&workspace.root, path, &entry.child, offset, limit)?;
    Ok(Value::object([
        ("path", Value::string(path)),
        ("offset", Value::number(offset)),
        ("before", before),
        ("after", after),
        ("review_token", Value::string(review.token)),
    ]))
}

fn page(
    root: &Path,
    name: &str,
    state: &files::Fingerprint,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    let size = match state {
        files::Fingerprint::File { size, .. } => *size,
        files::Fingerprint::Absent => {
            return Ok(Value::object([
                ("content", Value::string("")),
                ("size", Value::number(0)),
                ("next_offset", Value::Null),
            ]));
        }
        _ => return Err(format!("{name} is not a regular text file")),
    };
    if offset as u64 > size {
        return Err(format!("Offset exceeds {name} length"));
    }
    files::verify_content(root, name, state)?;
    let mut file = fs::File::open(files::path(root, name)?).map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(offset as u64))
        .map_err(|error| error.to_string())?;
    let mut bytes = vec![0u8; (size - offset as u64).min(limit as u64 + 3) as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    files::verify_content(root, name, state)?;
    let valid = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.len(),
        Err(error)
            if error.error_len().is_none() && offset as u64 + (bytes.len() as u64) < size =>
        {
            error.valid_up_to()
        }
        Err(_) => {
            return Err(format!(
                "{name} page is not UTF-8 text or offset splits a character"
            ));
        }
    };
    let text = std::str::from_utf8(&bytes[..valid]).map_err(|error| error.to_string())?;
    if text.contains('\0') {
        return Err(format!("{name} contains binary data"));
    }
    let keep = text.floor_char_boundary(limit);
    let next = offset + keep;
    Ok(Value::object([
        ("content", Value::string(&text[..keep])),
        ("size", Value::number(size)),
        (
            "next_offset",
            if (next as u64) < size {
                Value::number(next)
            } else {
                Value::Null
            },
        ),
    ]))
}
