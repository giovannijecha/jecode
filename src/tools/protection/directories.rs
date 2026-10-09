use super::*;

pub(super) struct Declaration {
    request: usize,
    reason: String,
    released: bool,
}

impl Protection {
    pub(super) fn has_earlier_directory(&self) -> bool {
        self.directories
            .values()
            .any(|scope| !scope.released && scope.request < self.request)
    }

    pub(super) fn remember_directories(
        &mut self,
        root: &Path,
        paths: &[&str],
        reason: &str,
    ) -> Result<(), String> {
        for path in paths {
            let path = files::existing_path(root, path)?;
            if path.is_dir() {
                self.declare_directory(
                    path.strip_prefix(root).unwrap().into(),
                    reason,
                    self.request,
                );
            }
        }
        Ok(())
    }

    fn declare_directory(&mut self, path: PathBuf, reason: &str, request: usize) {
        if self
            .directories
            .get(&path)
            .is_none_or(|scope| scope.released)
        {
            self.directories.insert(
                path,
                Declaration {
                    request,
                    reason: reason.into(),
                    released: false,
                },
            );
        }
    }

    pub(super) fn covering_directory(&self, path: &Path) -> Option<&Path> {
        if self.files.get(path).is_some_and(|entry| entry.released) {
            return None;
        }
        self.directories.iter().find_map(|(directory, scope)| {
            (!scope.released && scope.request < self.request && path.starts_with(directory))
                .then_some(directory.as_path())
        })
    }

    pub(super) fn directory_roots(&self) -> impl Iterator<Item = &Path> {
        self.directories
            .iter()
            .filter_map(|(path, scope)| (!scope.released).then_some(path.as_path()))
    }

    pub(super) fn directory_view(&self) -> Value {
        Value::Array(
            self.directories
                .iter()
                .map(|(path, scope)| {
                    Value::object([
                        (
                            "path",
                            Value::string(if path.as_os_str().is_empty() {
                                ".".into()
                            } else {
                                path.to_string_lossy().into_owned()
                            }),
                        ),
                        ("registered_request", Value::number(scope.request)),
                        ("reason", Value::string(&scope.reason)),
                        ("released", Value::Bool(scope.released)),
                    ])
                })
                .collect(),
        )
    }

    pub(super) fn release_directory(
        &mut self,
        root: &Path,
        path: &str,
        arguments: &Value,
    ) -> Result<Option<Value>, String> {
        let path = files::writable_path(root, path)?;
        let Some(scope) = self.directories.get_mut(path.strip_prefix(root).unwrap()) else {
            return Ok(None);
        };
        if scope.released {
            return Err("Directory preservation declaration is already released".into());
        }
        if self.request <= scope.request {
            return Err("A directory declaration can be released only by a newer user request explicitly permitting that scope change".into());
        }
        let reason = required_string(arguments, "reason")?;
        if reason.trim().is_empty() {
            return Err("reason must describe the newer user's directory scope decision".into());
        }
        scope.released = true;
        scope.reason = reason.into();
        Ok(Some(Value::object([
            ("released_directory_scope", Value::Bool(true)),
            ("individual_baselines_unchanged", Value::Bool(true)),
        ])))
    }

    pub(super) fn rebuild_directories(
        &mut self,
        message: &Value,
        result: &Value,
        calls: &BTreeMap<&str, &Value>,
    ) -> Result<(), String> {
        if result.get("error").is_some()
            || result.get("outcome").and_then(Value::as_str) == Some("unknown")
        {
            return Ok(());
        }
        let Some(function) = message
            .get("tool_call_id")
            .and_then(Value::as_str)
            .and_then(|id| calls.get(id))
            .and_then(|call| call.get("function"))
            .filter(|function| function.get("name").and_then(Value::as_str) == Some("protect"))
        else {
            return Ok(());
        };
        if let Some(scopes) = result
            .get("scope_review")
            .and_then(|view| view.get("directory_declarations"))
            .and_then(Value::as_array)
        {
            for value in scopes {
                let path = relative_scope(required_string(value, "path")?)
                    .ok_or("Invalid saved directory declaration path")?;
                self.directories.insert(
                    path,
                    Declaration {
                        request: value
                            .get("registered_request")
                            .and_then(Value::as_usize)
                            .ok_or("Invalid saved directory declaration request")?,
                        reason: required_string(value, "reason")?.into(),
                        released: value.get("released") == Some(&Value::Bool(true)),
                    },
                );
            }
        } else if let Some(arguments) = function
            .get("arguments")
            .and_then(Value::as_str)
            .and_then(|text| json::parse(text).ok())
            && arguments.get("action").and_then(Value::as_str) == Some("record")
        {
            self.recover_directory_arguments(
                &arguments,
                self.request,
                result
                    .get("file_protections")
                    .and_then(Value::as_array)
                    .unwrap_or(&[]),
            );
        }
        Ok(())
    }

    // Legacy successful directory records retain their declared scope from the native
    // call and its captured descendants, without inferring constraints from file names.
    pub(super) fn recover_directory_arguments(
        &mut self,
        arguments: &Value,
        request: usize,
        entries: &[Value],
    ) {
        let reason = arguments
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("");
        for path in arguments
            .get("paths")
            .and_then(Value::as_array)
            .unwrap_or(&[])
        {
            if let Some(path) = path.as_str().and_then(relative_scope)
                && entries
                    .iter()
                    .filter_map(|entry| entry.get("path").and_then(Value::as_str))
                    .any(|entry| Path::new(entry) != path && Path::new(entry).starts_with(&path))
            {
                self.declare_directory(path, reason, request);
            }
        }
    }
}

fn relative_scope(text: &str) -> Option<PathBuf> {
    let path = Path::new(text);
    let mut relative = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(value) => relative.push(value),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(relative)
}
