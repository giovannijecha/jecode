mod attachment;
mod files;
pub mod processes;
mod schema;
mod shell;

pub(crate) use files::read_text_page;
pub use schema::definitions;
pub use shell::find_bash;

use crate::cancel::Cancellation;
use crate::checkpoints::actions::{self, Caller};
use crate::checkpoints::session::{CommandGuard, Session};
use crate::checkpoints::show::{self, Content};
use crate::json::{self, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OUTPUT_LIMIT: usize = 64 * 1024;

pub struct Execution {
    pub result: Value,
    pub change: Option<crate::file_change::Change>,
}

/// Work done before a tool call runs, so it can be shown before the result.
#[derive(Default)]
pub struct Begun {
    command: Option<CommandGuard>,
}

impl Begun {
    /// The checkpoint before a shell command and how current it is.
    pub fn note(&self) -> Option<String> {
        self.command.as_ref().map(CommandGuard::note)
    }
}

pub struct Tools {
    root: PathBuf,
    bash: PathBuf,
    outputs: crate::output::Store,
    temporary: Option<crate::scratch::Area>,
    attachments: Option<crate::attachments::Pool>,
    processes: processes::Registry,
    checkpoints: Option<Arc<Session>>,
}

impl Tools {
    pub fn new(root: &Path) -> Result<Self, String> {
        let root = fs::canonicalize(root)
            .map_err(|error| format!("Invalid working directory: {error}"))?;
        if !root.is_dir() {
            return Err("The working directory must be a directory".into());
        }
        Ok(Self {
            outputs: crate::output::Store::new(
                root.join(".jecode-output"),
                crate::redact::Redactor::empty(),
            ),
            root,
            bash: shell::find_bash()?,
            temporary: None,
            attachments: None,
            processes: processes::Registry::default(),
            checkpoints: None,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn bash(&self) -> &Path {
        &self.bash
    }

    #[cfg(test)]
    pub fn configure_output(&mut self, directory: PathBuf, redactor: crate::redact::Redactor) {
        self.outputs = crate::output::Store::new(directory, redactor);
    }
    pub fn configure_session_output(
        &mut self,
        directory: PathBuf,
        session: &str,
        redactor: crate::redact::Redactor,
    ) -> Result<(), String> {
        self.outputs =
            crate::output::Store::for_session(directory, self.root.clone(), session, redactor)?;
        Ok(())
    }
    pub(crate) fn configure_output_store(&mut self, outputs: crate::output::Store) {
        self.outputs = outputs;
    }
    pub fn include_credentials(&mut self, redactor: crate::redact::Redactor) {
        self.outputs.include(redactor);
    }

    pub fn configure_attachments(&mut self, pool: crate::attachments::Pool) {
        self.attachments = Some(pool);
    }

    pub fn attachments(&self) -> Option<&crate::attachments::Pool> {
        self.attachments.as_ref()
    }

    /// Stops the persistent processes of the conversation that is ending.
    pub fn stop_processes(&self) {
        self.processes.stop_all();
    }

    /// Local inspection and stop control for the current conversation's processes.
    pub fn observe_processes(&self, redactor: crate::redact::Redactor) -> processes::Observer {
        self.processes.observer(redactor)
    }

    pub fn configure_temporary(&mut self, area: crate::scratch::Area) {
        self.temporary = Some(area);
    }

    pub fn temporary(&self) -> Result<&crate::scratch::Area, String> {
        self.temporary
            .as_ref()
            .ok_or("Session temporary storage is unavailable".into())
    }

    pub fn temporary_instructions(&self) -> String {
        self.temporary
            .as_ref()
            .map_or(String::new(), |area| area.instructions())
    }

    fn file_arguments(&self, arguments: &Value, create: bool) -> Result<(PathBuf, Value), String> {
        let path = required_string(arguments, "path")?;
        if path.starts_with("history:") {
            return Err("Session history is read-only and must be read through the agent".into());
        }
        if path.starts_with("skill:") {
            return Err("Personal skills are read-only and must be read through the agent".into());
        }
        if show::is_reference(path) {
            return Err(
                "Checkpoint history is read-only; restore it with the checkpoint tool".into(),
            );
        }
        let absolute = self
            .temporary
            .as_ref()
            .and_then(|area| area.relative_path(path));
        let Some(alias) = path.strip_prefix("tmp:").or(absolute.as_deref()) else {
            return Ok((self.root.clone(), arguments.clone()));
        };
        let (root, path) = self.temporary()?.file(alias, create)?;
        let mut arguments = arguments.clone();
        if let Value::Object(fields) = &mut arguments {
            fields.insert("path".into(), Value::string(path.to_string_lossy()));
        }
        Ok((root, arguments))
    }

    #[cfg(test)]
    pub fn execute(&self, name: &str, arguments: &str) -> Value {
        self.execute_with_cancel(name, arguments, &Cancellation::default())
    }

    #[cfg(test)]
    pub fn execute_with_cancel(
        &self,
        name: &str,
        arguments: &str,
        cancellation: &Cancellation,
    ) -> Value {
        self.execute_begun(self.begin(name, arguments), name, arguments, cancellation)
            .result
    }

    /// Records the checkpoint before a shell command, so its freshness can
    /// be shown while the command runs.
    pub fn begin(&self, name: &str, arguments: &str) -> Begun {
        let command = self
            .checkpoints
            .as_ref()
            .filter(|_| name == "bash")
            .and_then(|session| {
                let arguments = json::parse(arguments).ok()?;
                let text = arguments.get("command")?.as_str()?;
                Some(session.command(text))
            });
        Begun { command }
    }

    pub fn execute_begun(
        &self,
        begun: Begun,
        name: &str,
        arguments: &str,
        cancellation: &Cancellation,
    ) -> Execution {
        let result: Result<(Value, Option<crate::file_change::Change>), String> =
            json::parse(arguments).and_then(|arguments| {
                if !matches!(arguments, Value::Object(_)) {
                    return Err("Tool arguments must be a JSON object".into());
                }
                match name {
                    "read" => {
                        let path = required_string(&arguments, "path")?;
                        if attachment::is_reference(path) {
                            attachment::read(
                                self.attachments.as_ref(),
                                path,
                                &arguments,
                                cancellation,
                            )
                            .map(|value| (value, None))
                        } else if crate::output::is_reference(path) {
                            files::read_page(&self.outputs.resolve(path)?, &arguments, cancellation)
                                .map(|value| (value, None))
                        } else if show::is_reference(path) {
                            self.read_checkpoint(path, &arguments, cancellation)
                                .map(|value| (value, None))
                        } else {
                            let (root, arguments) = self.file_arguments(&arguments, false)?;
                            attachment::read_file(
                                self.attachments.as_ref(),
                                &root,
                                &arguments,
                                cancellation,
                            )
                            .map(|value| (value, None))
                        }
                    }
                    "write" | "edit" => {
                        let (root, arguments) = self.file_arguments(&arguments, name == "write")?;
                        // Session temporary files are outside the workspace scope.
                        let checkpoints = self.checkpoints.as_deref().filter(|_| root == self.root);
                        let result = if name == "write" {
                            files::write_with_change(&root, &arguments, checkpoints)
                        } else {
                            files::edit_with_change(&root, &arguments, checkpoints)
                        };
                        result.map(|(mut value, change)| {
                            if self.checkpoints.is_some()
                                && root != self.root
                                && value.get("bytes_written").is_some()
                                && let Value::Object(fields) = &mut value
                            {
                                fields.insert(
                                "checkpoint".into(),
                                Value::object([
                                    ("status", Value::string("not_tracked")),
                                    (
                                        "reason",
                                        Value::string(
                                            "session temporary storage is outside the workspace",
                                        ),
                                    ),
                                ]),
                            );
                            }
                            (value, change)
                        })
                    }
                    "bash" => {
                        let temporary = self
                            .temporary
                            .as_ref()
                            .map(|area| area.ensure())
                            .transpose()?;
                        let mut result = shell::execute(
                            &self.bash,
                            &self.root,
                            &arguments,
                            cancellation,
                            &self.outputs,
                            temporary.as_deref(),
                            &self.processes,
                        )
                        .unwrap_or_else(|error| Value::object([("error", Value::string(error))]));
                        if let Some(command) = begun.command {
                            let running = matches!(
                                result.get("status").and_then(Value::as_str),
                                Some("running" | "stopping")
                            );
                            let checkpoint = command.finish(running);
                            if let Value::Object(fields) = &mut result {
                                fields.insert("checkpoint".into(), checkpoint);
                            }
                        }
                        Ok((result, None))
                    }
                    "process" => self
                        .processes
                        .execute(&arguments, cancellation)
                        .map(|value| (value, None)),
                    "checkpoint" => self.checkpoint(&arguments).map(|value| (value, None)),
                    _ => Err(format!("Unknown tool: {name}")),
                }
            });
        let (result, change) =
            result.unwrap_or_else(|error| (Value::object([("error", Value::string(error))]), None));
        Execution { result, change }
    }

    /// Protects workspace changes from this conversation's tools.
    pub fn configure_checkpoints(&mut self, session: Arc<Session>) {
        let listener = Arc::clone(&session);
        self.processes
            .listen(Arc::new(move |running| listener.set_background(running)));
        self.checkpoints = Some(session);
    }

    pub fn checkpoints(&self) -> Option<&Arc<Session>> {
        self.checkpoints.as_ref()
    }

    fn enabled_checkpoints(&self) -> Result<&Arc<Session>, String> {
        self.checkpoints
            .as_ref()
            .ok_or_else(|| "Checkpoints are not enabled in this session".into())
    }

    fn checkpoint(&self, arguments: &Value) -> Result<Value, String> {
        let session = self.enabled_checkpoints()?;
        let origin = session.origin();
        let caller = Caller {
            origin: &origin,
            user: false,
        };
        let result = actions::run(session.workspace()?, &caller, arguments)?;
        if result.get("instructions_changed") == Some(&Value::Bool(true)) {
            session.restored_instructions();
        }
        Ok(result)
    }

    fn read_checkpoint(
        &self,
        path: &str,
        arguments: &Value,
        cancellation: &Cancellation,
    ) -> Result<Value, String> {
        let session = self.enabled_checkpoints()?;
        match show::content(session.workspace()?, path)? {
            Content::Text(text) => files::read_text_page(&text, arguments, cancellation),
            Content::Binary(bytes) => Ok(Value::object([
                ("binary", Value::Bool(true)),
                ("bytes", Value::number(bytes)),
                (
                    "note",
                    Value::string("restore the path to use this content"),
                ),
            ])),
            Content::Directory(entries) => Ok(Value::object([(
                "entries",
                Value::Array(entries.into_iter().map(Value::string).collect()),
            )])),
            Content::Link(target) => Ok(Value::object([("link", Value::string(target))])),
        }
    }
}

fn required_string<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

fn optional_number(
    arguments: &Value,
    name: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, String> {
    let Some(value) = arguments.get(name) else {
        return Ok(default);
    };
    let number = value
        .as_usize()
        .ok_or_else(|| format!("{name} must be an integer"))?;
    if !(min..=max).contains(&number) {
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(number)
}

#[cfg(test)]
mod checkpoint_tests;
#[cfg(test)]
mod local_read_tests;
#[cfg(test)]
mod tests;
