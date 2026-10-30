use super::{Mode, Workspace, files, tree};
use crate::disk::hash;
use crate::json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

mod evidence;
pub(super) use evidence::inspect_file;

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
    /// Copy entries this comparison cannot represent, such as sockets.
    unsupported: tree::Omitted,
}

impl Review {
    pub(super) fn value(self, workspace: &Workspace) -> Result<Value, String> {
        let mut changed = Vec::new();
        for entry in &self.entries {
            let mut value = entry.value();
            if let Value::Object(fields) = &mut value {
                fields.insert("evidence".into(), evidence::evidence(workspace, entry)?);
            }
            changed.push(value);
        }
        let mut value = Value::object([
            ("changed", Value::Array(changed)),
            ("review_token", Value::string(self.token)),
            (
                "scope",
                Value::string(
                    "Project entries excluding .git/.jecode metadata and root target/.tmp entries",
                ),
            ),
        ]);
        if let Value::Object(fields) = &mut value {
            if let Some(omitted) = workspace.omitted() {
                fields.insert("omitted_from_copy".into(), omitted);
            }
            if self.unsupported.count > 0 {
                fields.insert("unsupported_in_copy".into(), self.unsupported.value());
            }
        }
        Ok(value)
    }
}

pub(super) fn inspect(workspace: &Workspace) -> Result<Review, String> {
    if workspace.mode == Mode::Shared {
        return Ok(Review {
            entries: Vec::new(),
            token: "shared".into(),
            unsupported: tree::Omitted::default(),
        });
    }
    workspace.verify_owner()?;
    let (child_names, unsupported) = tree::inventory(&workspace.root)?;
    let mut names: BTreeSet<_> = workspace.baseline.keys().cloned().collect();
    names.extend(child_names);
    let mut entries = Vec::new();
    for name in names {
        files::validate_name(&name)?;
        if unsupported.names.contains(&name) {
            continue;
        }
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
    Ok(Review {
        entries,
        token,
        unsupported,
    })
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
    let mut directory_modes = Vec::new();
    let mut selected_entries: Vec<_> = review
        .entries
        .iter()
        .filter(|entry| selected.contains(&entry.path))
        .collect();
    selected_entries.sort_by(|left, right| {
        let deletion = |entry: &Entry| {
            matches!(entry.baseline, files::Fingerprint::Directory { .. })
                && matches!(entry.child, files::Fingerprint::Absent)
        };
        match (deletion(left), deletion(right)) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (true, true) => right
                .path
                .matches('/')
                .count()
                .cmp(&left.path.matches('/').count()),
            (false, false) => left
                .path
                .matches('/')
                .count()
                .cmp(&right.path.matches('/').count()),
        }
        .then_with(|| left.path.cmp(&right.path))
    });
    for entry in selected_entries {
        // Once a path is installed, report it with the failure.
        let observed = files::verify_content(
            &workspace.storage.join("base"),
            &entry.path,
            &entry.baseline,
        )
        .and_then(|()| {
            Ok((
                files::observe(&workspace.project, &entry.path)?,
                files::observe(&workspace.root, &entry.path)?,
            ))
        });
        let (current, child) = match observed {
            Ok(observed) => observed,
            Err(error) if applied.is_empty() => return Err(error),
            Err(error) => {
                failed.push(issue(&entry.path, &error));
                break;
            }
        };
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
        if let Err(error) = install(workspace, entry) {
            failed.push(issue(&entry.path, &error));
            break;
        }
        applied.push(Value::string(&entry.path));
        // The path is installed: from here a failure is reported with it.
        if let files::Fingerprint::Directory { mode } = entry.child {
            match files::observe(&workspace.project, &entry.path) {
                Ok(prepared) => directory_modes.push((entry.path.clone(), prepared, mode)),
                Err(error) => {
                    failed.push(issue(&entry.path, &error));
                    break;
                }
            }
        }
    }
    for (name, prepared, mode) in directory_modes.into_iter().rev() {
        match files::observe(&workspace.project, &name) {
            Ok(current) if current == prepared => {}
            Ok(_) => {
                failed.push(issue(&name, "Directory changed before applying its mode"));
                continue;
            }
            Err(error) => {
                failed.push(issue(&name, &error));
                continue;
            }
        }
        if let Err(error) = files::apply_mode(&files::path(&workspace.project, &name)?, mode) {
            failed.push(issue(&name, &error));
        }
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
        files::Fingerprint::Link { .. } => {
            Err("Link integration requires coordinator handling".into())
        }
        files::Fingerprint::Directory { mode } => {
            if !matches!(
                entry.current,
                files::Fingerprint::Absent | files::Fingerprint::Directory { .. }
            ) {
                return Err("Replacement of a non-directory requires coordinator handling".into());
            }
            let destination = files::path(&workspace.project, name)?;
            create_parents(&workspace.project, name)?;
            if files::observe(&workspace.project, name)? != entry.current
                || files::observe(&workspace.root, name)? != entry.child
            {
                return Err("Directory changed before installation".into());
            }
            if matches!(entry.current, files::Fingerprint::Absent) {
                fs::create_dir(&destination).map_err(|error| error.to_string())?;
            } else if let files::Fingerprint::Directory { mode: current_mode } = entry.current {
                #[cfg(unix)]
                let unblocks_children = current_mode & 0o200 == 0 && mode & 0o200 != 0;
                #[cfg(not(unix))]
                let unblocks_children = current_mode != 0 && *mode == 0;
                if unblocks_children {
                    files::apply_mode(&destination, *mode)?;
                }
            }
            Ok(())
        }
        files::Fingerprint::Absent => {
            let destination = files::path(&workspace.project, name)?;
            if files::observe(&workspace.project, name)? != entry.current {
                return Err("Project changed before deletion".into());
            }
            match entry.current {
                files::Fingerprint::Directory { .. } => {
                    fs::remove_dir(&destination).map_err(|error| error.to_string())?;
                }
                files::Fingerprint::File { .. } | files::Fingerprint::Absent => {
                    crate::checkpoints::files::remove(&destination)
                        .map_err(|error| error.to_string())?;
                }
                files::Fingerprint::Link { .. } => {
                    return Err("Deletion of a link requires coordinator handling".into());
                }
            }
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
