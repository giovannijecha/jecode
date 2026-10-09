use super::*;

impl Protection {
    pub(super) fn register(
        &mut self,
        root: &Path,
        store: &Store,
        arguments: &Value,
        paths: &[&str],
        cancellation: &Cancellation,
    ) -> Result<Value, String> {
        self.require_registration()?;
        if paths.is_empty() {
            return Err("Specify at least one existing project file or directory".into());
        }
        let reason = required_string(arguments, "reason")?;
        if reason.trim().is_empty() {
            return Err("reason must describe the user's preservation constraint".into());
        }
        let require_check = match arguments.get("require_check") {
            None => false,
            Some(Value::Bool(value)) => *value,
            _ => return Err("require_check must be a boolean".into()),
        };
        let files = expand(root, paths)?;
        let mut new = Vec::new();
        for path in &files {
            if self.files.get(path).is_some_and(|entry| !entry.released) {
                continue;
            }
            let id = crate::sessions::identifier();
            new.push((
                path.clone(),
                Entry {
                    snapshot: id,
                    baseline: fingerprint::capture(root, path, cancellation)?,
                    request: self.request,
                    reason: reason.into(),
                    require_check,
                    released: false,
                    history: None,
                },
            ));
        }
        self.save_intent(store, arguments, &new)?;
        for (path, entry) in new {
            self.files.insert(path, entry);
        }
        let mut errors = Vec::new();
        for path in files {
            let entry = &self.files[&path];
            let saved = store.protection_path(&entry.snapshot)?;
            if storage::verify_snapshot(&saved, &entry.baseline, cancellation).is_err() {
                if fingerprint::capture(root, &path, cancellation)? != entry.baseline {
                    errors.push(format!("{}: Original bytes are unavailable; the baseline cannot be replaced by the current file", path.display()));
                    continue;
                }
                if saved.exists() {
                    fs::remove_file(&saved).map_err(|error| error.to_string())?;
                }
                if let Err(error) =
                    storage::snapshot(root, &path, &saved, cancellation, &entry.baseline)
                {
                    errors.push(format!("{}: {error}", path.display()));
                }
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        self.remember_directories(root, paths, reason)?;
        Ok(Value::object([(
            "protected_file_count",
            Value::number(self.files.values().filter(|entry| !entry.released).count()),
        )]))
    }
}
