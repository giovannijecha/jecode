use super::Agent;
use crate::{
    collaboration::{Spec, workspace::Mode},
    json::Value,
    tools::Tools,
};
use std::{path::Path, time::Duration};

impl Agent {
    pub(super) fn agent_tool(&self, arguments: &Value) -> Result<Value, String> {
        if self.child {
            return Err("Collaborators cannot delegate or manage agents".into());
        }
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or("action must be a string")?;
        let observer = self.collaboration();
        let id = || {
            arguments
                .get("id")
                .and_then(Value::as_str)
                .ok_or("id must be a string")
        };
        match action {
            "create" => {
                let tasks = arguments
                    .get("tasks")
                    .and_then(Value::as_array)
                    .ok_or("tasks must be an array")?;
                if tasks.is_empty() || tasks.len() > 8 {
                    return Err("tasks must contain 1-8 assignments".into());
                }
                let tasks = tasks
                    .iter()
                    .map(|task| {
                        let title = required(task, "title")?.to_owned();
                        let instructions = required(task, "instructions")?.to_owned();
                        let context = optional_text(task, "context")?.unwrap_or("").to_owned();
                        let mode = match required(task, "workspace")? {
                            "shared" => Mode::Shared,
                            "isolated" => Mode::Isolated,
                            _ => return Err("workspace must be shared or isolated".into()),
                        };
                        Ok((title, instructions, context, mode))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let mut created = Vec::new();
                let gate =
                    std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
                let mut failure = None;
                for (title, instructions, context, mode) in tasks {
                    match observer.create(
                        self.tools.root(),
                        Spec {
                            title,
                            instructions,
                            context,
                            mode,
                        },
                        |root| self.new_child(root, mode),
                        gate.clone(),
                    ) {
                        Ok(value) => created.push(value),
                        Err(error) => {
                            failure = Some(error);
                            break;
                        }
                    }
                }
                if !created.is_empty() {
                    let titles = created
                        .iter()
                        .filter_map(|value| value.get("id").and_then(Value::as_str))
                        .map(str::to_owned)
                        .collect();
                    if let Err(error) = observer.receipt("created", None, 0, "Agents", titles) {
                        let ids = created
                            .iter()
                            .filter_map(|value| value.get("id").and_then(Value::as_str))
                            .map(str::to_owned)
                            .collect::<Vec<_>>();
                        observer.abort_created(&ids, &gate);
                        return Err(error);
                    }
                }
                let (lock, ready) = &*gate;
                *lock.lock().unwrap() = true;
                ready.notify_all();
                if let Some(error) = failure {
                    if created.is_empty() {
                        return Err(error);
                    }
                    return Ok(Value::object([
                        ("created", Value::Array(created)),
                        ("records", Value::Array(observer.snapshot())),
                        ("error", Value::string(error)),
                    ]));
                }
                Ok(Value::object([
                    ("created", Value::Array(created)),
                    ("records", Value::Array(observer.snapshot())),
                ]))
            }
            "update" => observer.update(
                id()?,
                required(arguments, "instructions")?.into(),
                optional_text(arguments, "context")?.unwrap_or("").into(),
            ),
            "list" => Ok(Value::object([(
                "records",
                Value::Array(observer.snapshot()),
            )])),
            "inspect" => {
                let id = id()?;
                observer
                    .snapshot()
                    .into_iter()
                    .find(|record| record.get("id").and_then(Value::as_str) == Some(id))
                    .ok_or("Unknown collaborator".into())
            }
            "wait" => {
                let timeout = optional_number(arguments, "timeout_ms")?.unwrap_or(60_000);
                if timeout > 60_000 {
                    return Err("timeout_ms must be at most 60000".into());
                }
                let filter = optional_text(arguments, "id")?;
                Ok(Value::object([(
                    "records",
                    Value::Array(observer.wait(
                        filter,
                        Duration::from_millis(timeout as u64),
                        &self.cancellation,
                    )?),
                )]))
            }
            "stop" => observer.stop(id()?),
            "review" => observer.review(
                id()?,
                arguments
                    .get("report_version")
                    .and_then(Value::as_usize)
                    .filter(|version| *version > 0)
                    .ok_or("report_version must be a positive integer")?,
                required(arguments, "note")?.into(),
            ),
            "diff" => {
                let workspace = observer.workspace(id()?)?;
                let path = optional_text(arguments, "path")?;
                let offset = optional_number(arguments, "offset")?.unwrap_or(0);
                let limit = optional_number(arguments, "limit")?.unwrap_or(16_384);
                if limit == 0 || limit > 65_536 {
                    return Err("limit must be 1-65536".into());
                }
                match path {
                    Some(path) => workspace.inspect_file(path, offset, limit),
                    None => workspace.inspect(),
                }
            }
            "integrate" => {
                let paths = arguments
                    .get("paths")
                    .and_then(Value::as_array)
                    .ok_or("paths must be an array")?
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .ok_or("paths must contain strings")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if paths.is_empty() {
                    return Err("paths must contain at least one file".into());
                }
                if self.cancellation.requested() {
                    return Err("Operation cancelled".into());
                }
                let checkpoint = self.tools.checkpoints().map_or_else(
                    || {
                        Value::object([
                            ("status", Value::string("none")),
                            ("reason", Value::string("Checkpoints are not enabled")),
                        ])
                    },
                    |session| {
                        session
                            .command_scoped_named_cancellable("agent", &paths, &self.cancellation)
                            .finish()
                    },
                );
                let mut result = observer
                    .workspace(id()?)?
                    .integrate(&paths, required(arguments, "expected_review_token")?)?;
                if let Value::Object(fields) = &mut result {
                    fields.insert("checkpoint".into(), checkpoint);
                }
                Ok(result)
            }
            "close" => observer.close(id()?),
            _ => Err(format!("Unknown agent action: {action}")),
        }
    }

    fn new_child(&self, root: &Path, mode: Mode) -> Result<Agent, String> {
        let mut backend = self.services().create(self.model().to_owned())?;
        backend.set_effort(self.effort());
        backend.set_routing(self.routing());
        backend.set_delegation(false);
        let mut child = Agent::with_backend(backend, Tools::new(root)?);
        child.child = true;
        child.accounts = self.accounts.clone();
        child.auto_compact = self.auto_compact;
        child.routing_preferences = self.routing_preferences.clone();
        child.redactor.include(self.redactor.clone());
        if mode == Mode::Isolated {
            child.inherited_project_instructions = Some(self.project_instructions.clone());
        }
        let handle = self
            .persistence
            .as_ref()
            .ok_or("Session storage is unavailable")?;
        let home = handle
            .store()
            .bucket()
            .parent()
            .and_then(Path::parent)
            .ok_or("Invalid session storage location")?
            .to_path_buf();
        let child_home = handle
            .store()
            .bucket()
            .join("collaboration")
            .join(handle.id())
            .join("child-home");
        crate::disk::ensure_private_directory(&child_home)
            .map_err(|error| format!("Could not prepare collaborator storage: {error}"))?;
        child.enable_sessions(&child_home)?;
        child.enable_checkpoints(&child_home);
        child.skills.configure(&home);
        child
            .tools
            .share_project_map_notes(&home, self.tools.root());
        Ok(child)
    }
}

fn required<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

fn optional_text<'a>(value: &'a Value, name: &str) -> Result<Option<&'a str>, String> {
    value
        .get(name)
        .map(|item| {
            item.as_str()
                .ok_or_else(|| format!("{name} must be a string"))
        })
        .transpose()
}

fn optional_number(value: &Value, name: &str) -> Result<Option<usize>, String> {
    value
        .get(name)
        .map(|item| {
            item.as_usize()
                .ok_or_else(|| format!("{name} must be an integer"))
        })
        .transpose()
}

#[cfg(test)]
mod checkpoint_tests;
#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod resume_tests;
#[cfg(test)]
mod tests;
