mod attachment;
mod files;
mod network;
pub mod processes;
mod schema;
mod shell;

pub(crate) use files::read_source;
pub(crate) use files::read_text_page;
pub use schema::definitions;
pub use shell::find_bash;

use crate::cancel::Cancellation;
use crate::checkpoints::actions;
use crate::checkpoints::session::{CommandGuard, Session};
use crate::checkpoints::show;
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
    browsers: crate::browser::Registry,
    browser_profiles: Option<PathBuf>,
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
            browsers: crate::browser::Registry::default(),
            browser_profiles: None,
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
        self.stop_browsers();
    }

    pub fn stop_browsers(&self) {
        self.browsers.stop_all();
    }

    pub fn configure_browser_profiles(&mut self, home: &Path) {
        self.browser_profiles = Some(home.join("browser/profiles"));
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
                "Saved recovery files are read-only; restore them with the checkpoint tool".into(),
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
        self.execute_begun(
            self.begin_cancellable(name, arguments, cancellation),
            name,
            arguments,
            cancellation,
        )
        .result
    }

    pub fn begin_cancellable(
        &self,
        name: &str,
        arguments: &str,
        cancellation: &Cancellation,
    ) -> Begun {
        let command = self
            .checkpoints
            .as_ref()
            .filter(|_| name == "bash")
            .and_then(|session| {
                let arguments = json::parse(arguments).ok()?;
                arguments.get("command")?.as_str()?;
                let paths = shell::protect_paths(&arguments).ok()?;
                Some(session.command_scoped_cancellable(&paths, cancellation))
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
                            files::write_with_change(&root, &arguments, checkpoints, cancellation)
                        } else {
                            files::edit_with_change(&root, &arguments, checkpoints, cancellation)
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
                                    ("status", Value::string("none")),
                                    ("saved_paths", Value::number(0)),
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
                            shell::Context {
                                bash: &self.bash,
                                root: &self.root,
                                outputs: &self.outputs,
                                temporary: temporary.as_deref(),
                                processes: &self.processes,
                            },
                            &arguments,
                            cancellation,
                        )
                        .unwrap_or_else(|error| Value::object([("error", Value::string(error))]));
                        if let Some(command) = begun.command {
                            let checkpoint = command.finish();
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
                    "checkpoint" => self
                        .checkpoint(&arguments, cancellation)
                        .map(|value| (value, None)),
                    "web" => crate::web::execute(self.temporary()?, &arguments, cancellation)
                        .map(|value| (value, None)),
                    "browser" => self
                        .browser(&arguments, cancellation)
                        .map(|value| (value, None)),
                    _ => Err(format!("Unknown tool: {name}")),
                }
            });
        let (result, change) =
            result.unwrap_or_else(|error| (Value::object([("error", Value::string(error))]), None));
        Execution { result, change }
    }

    /// Protects workspace changes from this conversation's tools.
    pub fn configure_checkpoints(&mut self, session: Arc<Session>) {
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

    fn checkpoint(&self, arguments: &Value, cancellation: &Cancellation) -> Result<Value, String> {
        let session = self.enabled_checkpoints()?;
        let result = actions::run(session.store()?, arguments, cancellation)?;
        if result
            .get("restored")
            .and_then(Value::as_array)
            .is_some_and(|paths| {
                paths.iter().any(|entry| {
                    entry
                        .get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| path.rsplit('/').next() == Some("JECODE.md"))
                })
            })
        {
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
        show::read(session.store()?, path, arguments, cancellation)
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
