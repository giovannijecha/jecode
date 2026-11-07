mod access;
mod attachment;
pub(crate) mod coerce;
mod configuration;
pub(crate) mod discovery;
pub(crate) mod edit;
mod exec;
mod files;
mod interactive;
mod lsp;
mod mcp;
mod network;
pub mod processes;
pub(crate) mod schema;
mod shell;
mod temporary;
mod writes;

pub(crate) use files::read_source;
pub(crate) use files::read_text_page;
#[cfg(test)]
pub use schema::encoded_size;
pub use schema::{definitions, definitions_for, encoded_size_for};

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
    /// An approved file access outside the working folder uses this
    /// directory as its root; see `Tools::access`.
    base: Option<PathBuf>,
    permission: Option<(
        crate::permissions::Level,
        Option<crate::permissions::Access>,
    )>,
}

impl Begun {
    pub fn with_base(mut self, base: Option<PathBuf>) -> Self {
        self.base = base;
        self
    }

    pub fn with_permission(
        mut self,
        level: crate::permissions::Level,
        access: Option<crate::permissions::Access>,
    ) -> Self {
        self.permission = Some((level, access));
        self
    }

    /// The checkpoint before a shell command and how current it is.
    pub fn note(&self) -> Option<String> {
        self.command.as_ref().map(CommandGuard::note)
    }
}

pub struct Tools {
    root: PathBuf,
    outputs: crate::output::Store,
    temporary: Option<crate::scratch::Area>,
    /// Temporary areas of the agents above this one, nearest first.
    inherited_temporary: Vec<crate::scratch::Area>,
    /// This agent's collaborators, whose temporary files it can read.
    collaborators: crate::collaboration::Observer,
    attachments: Option<crate::attachments::Pool>,
    processes: processes::Registry,
    checkpoints: Option<Arc<Session>>,
    browsers: crate::browser::Registry,
    browser_profiles: Option<PathBuf>,
    session_home: Option<PathBuf>,
    mcp_home: Option<PathBuf>,
    mcp_redactor: crate::redact::Redactor,
    mcp_runtime: crate::mcp::Runtime,
    lsp_home: Option<PathBuf>,
    lsp_runtime: crate::lsp::Runtime,
    lsp_redactor: crate::redact::Redactor,
    project_map: crate::project_map::Map,
}

impl Tools {
    pub fn new(root: &Path) -> Result<Self, String> {
        let root = fs::canonicalize(root)
            .map_err(|error| format!("Invalid working directory: {error}"))?;
        if !root.is_dir() {
            return Err("The working directory must be a directory".into());
        }
        Ok(Self {
            project_map: crate::project_map::Map::new(root.clone()),
            outputs: crate::output::Store::new(
                root.join(".jecode-output"),
                crate::redact::Redactor::empty(),
            ),
            root,
            temporary: None,
            inherited_temporary: Vec::new(),
            collaborators: crate::collaboration::Observer::default(),
            attachments: None,
            processes: processes::Registry::default(),
            checkpoints: None,
            browsers: crate::browser::Registry::default(),
            browser_profiles: None,
            session_home: None,
            mcp_home: None,
            mcp_redactor: crate::redact::Redactor::empty(),
            mcp_runtime: crate::mcp::Runtime::default(),
            lsp_home: None,
            lsp_runtime: crate::lsp::Runtime::default(),
            lsp_redactor: crate::redact::Redactor::empty(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
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
        temporary::own(path)?;
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
            .filter(|_| matches!(name, "bash" | "exec" | "process"))
            .and_then(|session| {
                let arguments = json::parse(arguments).ok()?;
                match name {
                    "bash" => {
                        arguments.get("command")?.as_str()?;
                    }
                    "exec" => {
                        arguments.get("executable")?.as_str()?;
                    }
                    _ if matches!(
                        arguments.get("action").and_then(Value::as_str),
                        Some("write" | "resize" | "interrupt")
                    ) => {}
                    _ => return None,
                }
                let paths = shell::protect_paths(&arguments).ok()?;
                Some(session.command_scoped_named_cancellable(name, &paths, cancellation))
            });
        Begun {
            command,
            base: None,
            permission: None,
        }
    }

    pub fn execute_begun(
        &self,
        begun: Begun,
        name: &str,
        arguments: &str,
        cancellation: &Cancellation,
    ) -> Execution {
        let base = begun.base.clone();
        let base = base.as_deref();
        let permission = begun.permission.as_ref();
        let result: Result<(Value, Option<crate::file_change::Change>), String> =
            json::parse(arguments).and_then(|arguments| {
                if !matches!(arguments, Value::Object(_)) {
                    return Err("Tool arguments must be a JSON object".into());
                }
                parallel(&arguments)?;
                match name {
                    "list" | "search" => discovery::execute(
                        base.unwrap_or(&self.root),
                        name,
                        &base.map_or_else(
                            || arguments.clone(),
                            |base| self.rebase(base, &arguments),
                        ),
                        cancellation,
                    )
                    .map(|value| (value, None)),
                    "map" => self
                        .project_map
                        .execute(&arguments, cancellation)
                        .map(|value| (value, None)),
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
                            let (root, arguments) = self.read_arguments(&arguments)?;
                            let (root, arguments) = match base {
                                Some(base) => (base.to_path_buf(), self.rebase(base, &arguments)),
                                None => (root, arguments),
                            };
                            attachment::read_file(
                                self.attachments.as_ref(),
                                &root,
                                &arguments,
                                cancellation,
                            )
                            .map(|value| (value, None))
                        }
                    }
                    "edit" if edit::several(&arguments) => {
                        let outside = permission
                            .and_then(|(_, access)| access.as_ref())
                            .is_some_and(|access| access.outside);
                        self.edit_files(&arguments, outside, cancellation)
                    }
                    "write" | "edit" => self.change_file(name, &arguments, base, cancellation),
                    "bash" | "exec" | "process" => {
                        let temporary = self
                            .temporary
                            .as_ref()
                            .filter(|_| name != "process")
                            .map(|area| area.ensure())
                            .transpose()?;
                        let mut sandbox = if name == "process" {
                            None
                        } else if let Some((level, access)) = permission {
                            crate::sandbox::Policy::for_command(
                                *level,
                                &self.root,
                                temporary.as_deref(),
                                access.as_ref(),
                            )?
                        } else {
                            None
                        };
                        if let Some(policy) = &mut sandbox {
                            policy.protect_session_home(self.session_home.as_deref())?;
                            policy.allow_installed_rust_tools(self.session_home.as_deref())?;
                            policy.allow_installed_node_tools(self.session_home.as_deref())?;
                            policy.allow_path_tools(self.session_home.as_deref())?;
                            if name == "bash" {
                                policy.allow_owned_shell()?;
                            }
                        }
                        let context = shell::Context {
                            root: &self.root,
                            outputs: &self.outputs,
                            temporary: temporary.as_deref(),
                            processes: &self.processes,
                            sandbox: sandbox.as_ref(),
                        };
                        let mut result = match name {
                            "bash" => shell::execute(context, &arguments, cancellation),
                            "process" => self.processes.execute(
                                &arguments,
                                cancellation,
                                self.outputs.redactor(),
                            ),
                            _ => match arguments.get("tty") {
                                None | Some(Value::Bool(false)) => {
                                    exec::execute(context, &arguments, cancellation)
                                }
                                Some(Value::Bool(true)) => {
                                    interactive::execute(context, &arguments, cancellation)
                                }
                                _ => Err("tty must be a boolean".into()),
                            },
                        }
                        .unwrap_or_else(|error| Value::object([("error", Value::string(error))]));
                        if let Some(command) = begun.command {
                            let checkpoint = command.finish();
                            if let Value::Object(fields) = &mut result {
                                fields.insert("checkpoint".into(), checkpoint);
                            }
                        }
                        Ok((result, None))
                    }
                    "lsp" => Ok((self.lsp(&arguments, cancellation), None)),
                    "mcp" => Ok((
                        crate::mcp::execute_with(
                            &self.mcp_runtime,
                            self.mcp_home.as_deref(),
                            &arguments,
                            cancellation,
                            &self.mcp_redactor,
                            Some(&self.outputs),
                        ),
                        None,
                    )),
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
        // Reconcile actual disk state independently of presentation events.
        // Commands may change files even when they return a failure.
        let refresh_analysis = matches!(name, "write" | "edit" | "bash" | "exec")
            || matches!(name, "process" | "checkpoint")
                && json::parse(arguments).ok().is_some_and(|arguments| {
                    arguments.get("action").and_then(Value::as_str)
                        == Some(if name == "process" {
                            "write"
                        } else {
                            "restore"
                        })
                });
        if refresh_analysis {
            self.lsp_runtime.invalidate_paths(&[]);
        }
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
                        .is_some_and(|path| path.rsplit('/').next() == Some("AGENTS.md"))
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

pub(crate) fn parallel(arguments: &Value) -> Result<Option<bool>, String> {
    match arguments.get("parallel") {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err("parallel must be a boolean".into()),
    }
}

pub(crate) fn required_string<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

pub(crate) fn optional_number(
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
mod multi_edit_tests;
#[cfg(test)]
mod tests;
