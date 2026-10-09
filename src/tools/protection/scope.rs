use super::*;

impl Protection {
    pub fn scope_refusal(&mut self, root: &Path, cancellation: &Cancellation) -> Option<Value> {
        if self.reviewed_request == Some(self.request)
            || (self.files.values().all(|entry| entry.released) && !self.has_earlier_directory())
        {
            return None;
        }
        self.scope_blocked = true;
        Some(Value::object([
            (
                "error",
                Value::string(
                    "Project operation not executed. Review the current preservation scope: record newly covered paths, or protect status with scope_exclusions and reason for explicitly editable/uncovered related files. A status call alone does not approve unclassified candidates. Do not exclude a test merely because it was created earlier, or protect editable source merely because it is listed. Incomplete inventories require project inspection and a scope reason before retrying.",
                ),
            ),
            ("outcome", Value::string("not_started")),
            ("scope_review", self.scope_view(root, cancellation)),
        ]))
    }

    pub(super) fn scope_view(&self, root: &Path, cancellation: &Cancellation) -> Value {
        let mut pending = self
            .files
            .iter()
            .filter(|(_, entry)| !entry.released)
            .filter_map(|(path, _)| {
                path.parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
            })
            .chain(self.directory_roots())
            .map(|path| root.join(path))
            .collect::<BTreeSet<_>>();
        let mut visited = BTreeSet::new();
        let mut related = BTreeSet::new();
        let mut complete = true;
        while let Some(path) = pending.pop_first() {
            if cancellation.requested() || visited.len() >= 512 || related.len() >= 64 {
                complete = false;
                break;
            }
            let Ok(path) = files::existing_path(root, &path.to_string_lossy()) else {
                complete = false;
                continue;
            };
            if !visited.insert(path.clone()) {
                continue;
            }
            if path.is_file() {
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                if self.files.get(&relative).is_none_or(|entry| entry.released) {
                    related.insert(relative);
                }
            } else if let Ok(entries) = fs::read_dir(path) {
                for entry in entries {
                    if pending.len().saturating_add(visited.len()) >= 512 {
                        complete = false;
                        break;
                    }
                    match entry {
                        Ok(entry) if entry.file_type().is_ok_and(|kind| !kind.is_symlink()) => {
                            pending.insert(entry.path());
                        }
                        _ => complete = false,
                    }
                }
            } else {
                complete = false;
            }
        }
        let covered = related
            .iter()
            .filter(|path| self.covering_directory(path).is_some())
            .map(|path| Value::string(path.to_string_lossy()))
            .collect();
        let unclassified = related
            .iter()
            .filter(|path| !self.scope_exclusions.contains(*path))
            .map(|path| Value::string(path.to_string_lossy()))
            .collect();
        Value::object([
            ("request", Value::number(self.request)),
            (
                "reviewed",
                Value::Bool(self.reviewed_request == Some(self.request)),
            ),
            (
                "related_unregistered_files",
                Value::Array(
                    related
                        .into_iter()
                        .map(|path| Value::string(path.to_string_lossy()))
                        .collect(),
                ),
            ),
            ("related_inventory_complete", Value::Bool(complete)),
            ("unclassified_related_files", Value::Array(unclassified)),
            ("covered_by_recorded_directory", Value::Array(covered)),
            ("directory_declarations", self.directory_view()),
            (
                "scope_exclusions",
                Value::Array(
                    self.scope_exclusions
                        .iter()
                        .map(|path| Value::string(path.to_string_lossy()))
                        .collect(),
                ),
            ),
            ("scope_reason", Value::string(&self.scope_reason)),
            (
                "instruction",
                Value::string(
                    "This bounded inventory covers non-root parents, not the whole workspace. Record covered_by_recorded_directory files before mutations: the directory declaration carries into later requests even though new files were editable in their creating request. Status exclusions cannot waive this declaration or active baselines. Other candidates need record or explicit scope_exclusions/reason. A newer user decision permitting a scope change may release the directory declaration; its existing individual baselines remain active. Inspect incomplete inventories and explain that review with reason.",
                ),
            ),
        ])
    }

    pub(super) fn rebuild_scope(
        &mut self,
        message: &Value,
        result: &Value,
        calls: &BTreeMap<&str, &Value>,
    ) {
        if result.get("outcome").and_then(Value::as_str) == Some("not_started")
            && result.get("scope_review").is_some()
        {
            self.scope_blocked = true;
        }
        if result.get("error").is_none()
            && result.get("outcome").and_then(Value::as_str) != Some("unknown")
            && message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .and_then(|id| calls.get(id))
                .and_then(|call| call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                == Some("protect")
        {
            if let Some(view) = result.get("scope_review") {
                self.scope_exclusions = view
                    .get("scope_exclusions")
                    .and_then(Value::as_array)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|path| path.as_str().map(PathBuf::from))
                    .filter(|path| self.covering_directory(path).is_none())
                    .collect();
                self.scope_reason = view
                    .get("scope_reason")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into();
                let classified = view
                    .get("related_unregistered_files")
                    .and_then(Value::as_array)
                    .unwrap_or(&[])
                    .iter()
                    .all(|path| {
                        path.as_str()
                            .is_some_and(|path| self.scope_exclusions.contains(Path::new(path)))
                    });
                let earlier = self
                    .files
                    .values()
                    .any(|entry| !entry.released && entry.request < self.request)
                    || self.has_earlier_directory();
                if view.get("reviewed") == Some(&Value::Bool(true)) && (!earlier || classified) {
                    self.reviewed_request = Some(self.request);
                    self.scope_blocked = false;
                }
            } else {
                // Legacy receipts are reconsidered on the next user request.
                self.reviewed_request = Some(self.request);
                self.scope_blocked = false;
            }
        }
    }
}
