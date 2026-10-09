use super::*;
use std::fs::OpenOptions;
use std::hash::Hasher;
use std::io::Write;

impl Protection {
    pub(super) fn save_intent(
        &self,
        store: &Store,
        arguments: &Value,
        entries: &[(PathBuf, Entry)],
    ) -> Result<(), String> {
        if entries.is_empty() {
            return Ok(());
        }
        let intent = Value::object([
            ("arguments", arguments.clone()),
            ("request", Value::number(self.request)),
            (
                "entries",
                Value::Array(
                    entries
                        .iter()
                        .map(|(path, entry)| entry.value(path))
                        .collect(),
                ),
            ),
        ]);
        let path = store.protection_intent_path(&crate::sessions::identifier())?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|error| error.to_string())?;
        let sealed = Value::object([
            ("fingerprint", Value::string(integrity(&intent))),
            ("intent", intent),
        ]);
        file.write_all(sealed.encode().as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())
    }

    pub(super) fn release_pending(
        &mut self,
        path: &str,
        arguments: &Value,
    ) -> Result<bool, String> {
        let Some(index) = self
            .pending
            .iter()
            .position(|(_, at, _)| path == format!("Incomplete registration at history:{at}"))
        else {
            return Ok(false);
        };
        if self.request <= self.pending[index].0 {
            return Err("An incomplete registration can be released only after a newer user request explicitly accepts the unavailable original version".into());
        }
        if required_string(arguments, "reason")?.trim().is_empty() {
            return Err("reason must describe the newer user's decision".into());
        }
        self.pending.remove(index);
        Ok(true)
    }

    pub fn recover(&mut self, _root: &Path, store: &Store) -> Result<(), String> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let mut recovered = BTreeSet::new();
        let mut directories = Vec::new();
        for path in store.protection_intents()? {
            // A crash can leave a partial intent. Keep its registration unknown;
            // accepting current bytes would silently replace the missing original.
            let Some(sealed) = fs::read_to_string(path)
                .ok()
                .and_then(|text| json::parse(&text).ok())
            else {
                continue;
            };
            let Some(intent) = sealed.get("intent").filter(|intent| {
                sealed.get("fingerprint").and_then(Value::as_str)
                    == Some(integrity(intent).as_str())
            }) else {
                continue;
            };
            for (request, at, arguments) in &self.pending {
                if intent.get("request").and_then(Value::as_usize) != Some(*request)
                    || intent.get("arguments") != Some(arguments)
                {
                    continue;
                }
                let parsed = (|| {
                    let mut parsed = Vec::new();
                    for value in intent
                        .get("entries")
                        .and_then(Value::as_array)
                        .ok_or("Invalid preservation registration intent")?
                    {
                        let path = PathBuf::from(required_string(value, "path")?);
                        if path.as_os_str().is_empty()
                            || path
                                .components()
                                .any(|part| !matches!(part, Component::Normal(_)))
                        {
                            return Err("Invalid intent path".into());
                        }
                        let snapshot = required_string(value, "snapshot_id")?;
                        if !crate::sessions::valid_id(snapshot) {
                            return Err("Invalid intent baseline identifier".into());
                        }
                        let baseline = value
                            .get("baseline")
                            .and_then(fingerprint::version)
                            .ok_or("Invalid intent baseline version")?;
                        if baseline.get("state").and_then(Value::as_str) != Some("present") {
                            return Err("Invalid intent baseline state".into());
                        }
                        parsed.push((
                            path,
                            Entry {
                                snapshot: snapshot.into(),
                                baseline,
                                request: *request,
                                reason: required_string(value, "reason")?.into(),
                                require_check: value.get("require_check")
                                    == Some(&Value::Bool(true)),
                                released: false,
                                history: Some(*at),
                            },
                        ));
                    }
                    if parsed.is_empty() {
                        return Err("Empty preservation intent".into());
                    }
                    Ok::<_, String>(parsed)
                })();
                let Ok(entries) = parsed else { continue };
                directories.push((
                    arguments.clone(),
                    *request,
                    entries
                        .iter()
                        .map(|(path, entry)| entry.value(path))
                        .collect::<Vec<_>>(),
                ));
                for (path, entry) in entries {
                    if self.files.get(&path).is_none_or(|old| old.released) {
                        self.files.insert(path, entry);
                    }
                }
                recovered.insert(*at);
            }
        }
        // An intent covers every selected path even when its baseline copy is incomplete.
        // Such entries stay unknown and can be retried only against the declared version.
        self.pending.retain(|(_, at, _)| !recovered.contains(at));
        for (arguments, request, entries) in directories {
            self.recover_directory_arguments(&arguments, request, &entries);
        }
        Ok(())
    }
}

// Detect incomplete or corrupted private metadata, not malicious tampering.
fn integrity(intent: &Value) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    hash.write(intent.encode().as_bytes());
    format!("rust-default:{:016x}", hash.finish())
}

impl Entry {
    pub(super) fn value(&self, path: &Path) -> Value {
        Value::object([
            ("path", Value::string(path.to_string_lossy())),
            ("snapshot_id", Value::string(&self.snapshot)),
            ("baseline", self.baseline.clone()),
            ("registered_request", Value::number(self.request)),
            ("reason", Value::string(&self.reason)),
            ("require_check", Value::Bool(self.require_check)),
        ])
    }
}
