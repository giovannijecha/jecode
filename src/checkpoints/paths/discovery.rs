use super::*;

pub fn discover(
    root: &Path,
    requested: &[String],
    excluded: &[String],
    cancel: &Cancellation,
) -> Result<Selection, String> {
    discover_with_rules(root, requested, excluded, cancel, None)
}

pub fn discover_saved(
    root: &Path,
    requested: &[String],
    excluded: &[String],
    cancel: &Cancellation,
    rules: &IgnoreRules,
) -> Result<Selection, String> {
    discover_with_rules(
        root,
        requested,
        excluded,
        cancel,
        Some(Ignore::saved(rules)),
    )
}

fn discover_with_rules(
    root: &Path,
    requested: &[String],
    excluded: &[String],
    cancel: &Cancellation,
    mut rules: Option<Ignore>,
) -> Result<Selection, String> {
    let requested: Vec<String> = requested
        .iter()
        .map(|path| normalized_request(path))
        .collect();
    let mut paths = BTreeSet::new();
    let mut roots = BTreeSet::new();
    let mut ignored = BTreeSet::new();
    let mut partial = Vec::new();
    for request in &requested {
        if cancel.requested() {
            return Err("Recovery discovery was cancelled".into());
        }
        // A descendant shadowed by an explicitly requested file/link ancestor
        // is logically absent in that tree. Capture records the ancestor first
        // and may then save this absence without following a link target.
        if !Path::new(request).is_absolute() && request.split('/').all(files::normal_component) {
            let paired = requested.iter().any(|ancestor| {
                !ancestor.is_empty()
                    && request
                        .strip_prefix(ancestor)
                        .is_some_and(|rest| rest.starts_with('/'))
                    && fs::symlink_metadata(absolute(root, ancestor))
                        .is_ok_and(|metadata| !ordinary_dir(&metadata))
            });
            if paired && protected(request, excluded) {
                paths.insert(request.clone());
                continue;
            }
        }
        // Whether the final spelling already passed `confined_parent`.
        let (name, confined) = if request.is_empty() || request == "." {
            (String::new(), true)
        } else if Path::new(request).is_absolute() {
            if Path::new(request)
                .components()
                .any(|part| part == Component::ParentDir)
            {
                partial.push((
                    request.clone(),
                    format!("Invalid workspace path: {request}"),
                ));
                continue;
            }
            let mut checked = None;
            if let Ok(rest) = Path::new(request).strip_prefix(root) {
                let lexical = rest
                    .components()
                    .filter_map(|part| match part {
                        Component::Normal(name) => name.to_str(),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("/");
                if !lexical.is_empty()
                    && let Err(error) = files::confined_parent(root, &lexical)
                {
                    partial.push((request.clone(), error));
                    continue;
                }
                checked = Some(lexical);
            }
            let Some(name) = relative(root, Path::new(request)) else {
                partial.push((
                    request.clone(),
                    format!("Path outside workspace: {request}"),
                ));
                continue;
            };
            let confined = checked.as_ref() == Some(&name);
            (name, confined)
        } else {
            let requested_path = absolute(root, request);
            if !request.split('/').all(files::normal_component) {
                partial.push((
                    request.clone(),
                    format!("Invalid workspace path: {request}"),
                ));
                continue;
            }
            if let Err(error) = files::confined_parent(root, request) {
                partial.push((request.clone(), error));
                continue;
            }
            let Some(name) = relative(root, &requested_path) else {
                partial.push((
                    request.clone(),
                    format!("Path outside workspace: {request}"),
                ));
                continue;
            };
            let confined = name == *request;
            (name, confined)
        };
        if !name.is_empty() && !protected(&name, excluded) {
            partial.push((name, "Internal recovery storage is excluded".into()));
            continue;
        }
        if !confined && let Err(error) = files::confined_parent(root, &name) {
            partial.push((name, error));
            continue;
        }
        let path = absolute(root, &name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                partial.push((name.clone(), format!("Could not inspect path: {error}")));
                continue;
            }
        };
        if !name.is_empty() {
            paths.insert(name.clone());
        }
        if metadata.as_ref().is_some_and(ordinary_dir) || name.is_empty() {
            let before = partial.len();
            let mut pending = vec![name.clone()];
            while let Some(directory) = pending.pop() {
                if cancel.requested() {
                    return Err("Recovery discovery was cancelled".into());
                }
                let directory_path = absolute(root, &directory);
                let entries = match fs::read_dir(&directory_path) {
                    Ok(entries) => entries,
                    Err(error) => {
                        partial.push((directory, format!("Could not traverse directory: {error}")));
                        continue;
                    }
                };
                let mut has_rules = false;
                let mut directories = Vec::new();
                for entry in entries {
                    if cancel.requested() {
                        return Err("Recovery discovery was cancelled".into());
                    }
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(error) => {
                            partial.push((
                                directory.clone(),
                                format!("Could not read directory entry: {error}"),
                            ));
                            continue;
                        }
                    };
                    let Some(part) = entry.file_name().to_str().map(str::to_owned) else {
                        partial.push((
                            directory.clone(),
                            "A directory entry has an unsupported name".into(),
                        ));
                        continue;
                    };
                    has_rules |= part.eq_ignore_ascii_case(".gitignore");
                    let child = if directory.is_empty() {
                        part
                    } else {
                        format!("{directory}/{part}")
                    };
                    if !eligible_beneath(&child, &name, excluded) {
                        continue;
                    }
                    // Like `symlink_metadata`, and free on Windows.
                    let metadata = match entry.metadata() {
                        Ok(metadata) => metadata,
                        Err(error) => {
                            partial.push((child, format!("Could not inspect entry: {error}")));
                            continue;
                        }
                    };
                    let traverse = ordinary_dir(&metadata);
                    #[cfg(windows)]
                    if metadata.is_dir() && !traverse {
                        use std::os::windows::fs::MetadataExt;
                        if metadata.file_attributes() & 0x400 != 0
                            && !metadata.file_type().is_symlink()
                        {
                            partial.push((
                                child.clone(),
                                "Reparse directory was not traversed".into(),
                            ));
                        }
                    }
                    if traverse {
                        directories.push(child);
                    } else {
                        paths.insert(child);
                    }
                }
                let rules = rules.get_or_insert_with(|| Ignore::new(root));
                rules.visit(&directory, has_rules);
                for child in directories {
                    if rules.ignored(&child) {
                        ignored.insert(child);
                    } else {
                        paths.insert(child.clone());
                        pending.push(child);
                    }
                }
            }
            if partial.len() == before {
                roots.insert(name);
            }
        } else if metadata.is_none() {
            // The named entry was absent. That is complete evidence that all
            // descendants were absent too, if this entry later becomes a tree.
            roots.insert(name);
        } else if metadata.as_ref().is_some_and(|m| m.is_dir()) {
            partial.push((name, "Directory was not traversed".into()));
        }
    }
    let ignore_rules = if roots.is_empty() {
        IgnoreRules::default()
    } else {
        rules.unwrap_or_else(|| Ignore::new(root)).snapshot(&roots)
    };
    Ok(Selection {
        paths: paths.into_iter().collect(),
        roots: roots.into_iter().collect(),
        ignored: ignored.into_iter().collect(),
        ignore_rules,
        partial,
    })
}
