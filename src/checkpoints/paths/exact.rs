use super::*;

/// Select only the named entries for a restore undo. A directory here is one
/// entry, never proof that its children were observed or saved.
pub fn discover_exact(
    root: &Path,
    requested: &[String],
    excluded: &[String],
    cancel: &Cancellation,
) -> Result<Selection, String> {
    let requested: Vec<String> = requested
        .iter()
        .map(|path| normalized_request(path))
        .collect();
    let mut paths = BTreeSet::new();
    let mut partial = Vec::new();
    for request in &requested {
        if cancel.requested() {
            return Err("Recovery discovery was cancelled".into());
        }
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
        let lexical = if Path::new(request).is_absolute() {
            match relative(root, Path::new(request)) {
                Some(name) => name,
                None => {
                    partial.push((
                        request.clone(),
                        format!("Path outside workspace: {request}"),
                    ));
                    continue;
                }
            }
        } else {
            if !request.split('/').all(files::normal_component) {
                partial.push((
                    request.clone(),
                    format!("Invalid workspace path: {request}"),
                ));
                continue;
            }
            request.clone()
        };
        let paired = requested.iter().any(|ancestor| {
            lexical
                .strip_prefix(ancestor)
                .is_some_and(|rest| rest.starts_with('/'))
                && fs::symlink_metadata(absolute(root, ancestor))
                    .is_ok_and(|metadata| !ordinary_dir(&metadata))
        });
        if !paired && let Err(error) = files::confined_parent(root, &lexical) {
            partial.push((lexical, error));
            continue;
        }
        let name = if paired {
            lexical
        } else {
            let Some(name) = relative(root, &absolute(root, &lexical)) else {
                partial.push((
                    lexical.clone(),
                    format!("Path outside workspace: {lexical}"),
                ));
                continue;
            };
            // Only a different live spelling needs its parents checked again.
            if name != lexical
                && let Err(error) = files::confined_parent(root, &name)
            {
                partial.push((name, error));
                continue;
            }
            name
        };
        if !protected(&name, excluded) {
            partial.push((name, "Internal recovery storage is excluded".into()));
            continue;
        }
        paths.insert(name);
    }
    Ok(Selection {
        paths: paths.into_iter().collect(),
        partial,
        ..Selection::default()
    })
}
