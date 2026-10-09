mod files;
mod protection;
mod schema;
mod shell;
mod watch;

pub(crate) use files::read_text_page;
pub use schema::definitions;
pub use shell::find_bash;

use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub(super) const OUTPUT_LIMIT: usize = 64 * 1024;

pub struct Tools {
    root: PathBuf,
    bash: PathBuf,
    outputs: crate::output::Store,
    temporary: Option<crate::scratch::Area>,
    watches: Mutex<watch::Watch>,
    protections: Mutex<protection::Protection>,
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
            watches: Mutex::new(watch::Watch::default()),
            protections: Mutex::new(protection::Protection::default()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn reset_watches(&self) {
        self.watches.lock().unwrap().reset();
        self.protections.lock().unwrap().reset();
    }

    pub(crate) fn record_file_history(&self, at: usize, result: &Value) {
        self.watches.lock().unwrap().record(at, result);
        self.protections.lock().unwrap().record(at, result);
    }

    pub(crate) fn restore_watches(&self, messages: &[Value]) -> Result<(), String> {
        self.watches.lock().unwrap().rebuild(messages);
        self.protections.lock().unwrap().rebuild(messages)
    }

    pub(crate) fn begin_request(&self, at: usize) {
        self.protections.lock().unwrap().begin_request(at);
    }

    pub(crate) fn protection_status(&self, cancellation: &Cancellation) -> Vec<Value> {
        self.protections
            .lock()
            .unwrap()
            .status(&self.root, &self.outputs, cancellation)
    }

    pub(crate) fn recover_protections(&self) -> Result<(), String> {
        self.protections
            .lock()
            .unwrap()
            .recover(&self.root, &self.outputs)
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

    pub fn execute_with_cancel(
        &self,
        name: &str,
        arguments: &str,
        cancellation: &Cancellation,
    ) -> Value {
        let result = json::parse(arguments).and_then(|arguments| {
            if !matches!(arguments, Value::Object(_)) {
                return Err("Tool arguments must be a JSON object".into());
            }
            match name {
                "read" => {
                    let path = required_string(&arguments, "path")?;
                    if crate::output::is_reference(path) {
                        files::read_page(&self.outputs.resolve(path)?, &arguments, cancellation)
                    } else {
                        let (root, arguments) = self.file_arguments(&arguments, false)?;
                        let mut result = files::read(&root, &arguments, cancellation)?;
                        self.observe_file(
                            &root,
                            &arguments,
                            "before_file_tool",
                            cancellation,
                            &mut result,
                        );
                        Ok(result)
                    }
                }
                "write" | "edit" => {
                    let (root, arguments) = self.file_arguments(&arguments, name == "write")?;
                    if root == self.root {
                        if let Some(refusal) = self
                            .protections
                            .lock()
                            .unwrap()
                            .scope_refusal(&root, cancellation)
                        {
                            return Ok(refusal);
                        }
                        self.protections
                            .lock()
                            .unwrap()
                            .refuse_write(&root, required_string(&arguments, "path")?)?;
                    }
                    let result = if name == "write" {
                        files::write(&root, &arguments)
                    } else {
                        files::edit(&root, &arguments)
                    };
                    result.map(|mut result| {
                        self.observe_file(
                            &root,
                            &arguments,
                            "file_tool",
                            cancellation,
                            &mut result,
                        );
                        result
                    })
                }
                "bash" => {
                    self.protections.lock().unwrap().require_registration()?;
                    if let Some(refusal) = self
                        .protections
                        .lock()
                        .unwrap()
                        .scope_refusal(&self.root, cancellation)
                    {
                        return Ok(refusal);
                    }
                    let watched = match arguments.get("watch") {
                        None => Vec::new(),
                        Some(Value::Array(paths)) => paths
                            .iter()
                            .map(|path| {
                                let path = path
                                    .as_str()
                                    .ok_or("watch must be an array of project file paths")?;
                                if path.starts_with("tmp:")
                                    || crate::output::is_reference(path)
                                    || path.starts_with("history:")
                                {
                                    return Err("watch accepts existing project files only".into());
                                }
                                let path = files::existing_path(&self.root, path)?;
                                if !path.is_file() {
                                    return Err(
                                        "watch accepts existing regular project files only".into(),
                                    );
                                }
                                Ok(path)
                            })
                            .collect::<Result<Vec<_>, String>>()?,
                        _ => return Err("watch must be an array of project file paths".into()),
                    };
                    let temporary = self
                        .temporary
                        .as_ref()
                        .map(|area| area.ensure())
                        .transpose()?;
                    let mut report = watch::Report::default();
                    {
                        let mut watches = self.watches.lock().unwrap();
                        watches.scan(&self.root, "before_command", cancellation, &mut report);
                        for path in watched {
                            watches.observe(
                                &self.root,
                                &path,
                                "before_command",
                                cancellation,
                                &mut report,
                            );
                        }
                    }
                    let mut result = shell::execute(
                        &self.bash,
                        &self.root,
                        &arguments,
                        cancellation,
                        &self.outputs,
                        temporary.as_deref(),
                    )
                    .unwrap_or_else(|error| Value::object([("error", Value::string(error))]));
                    let mut watches = self.watches.lock().unwrap();
                    watches.scan(&self.root, "during_command", cancellation, &mut report);
                    watches.attach(&mut result, report);
                    Ok(result)
                }
                "protect" => {
                    let mut result = self.protections.lock().unwrap().execute(
                        &self.root,
                        &self.outputs,
                        &arguments,
                        cancellation,
                    );
                    let paths = self.protections.lock().unwrap().active_paths(&self.root);
                    let mut report = watch::Report::default();
                    let mut watches = self.watches.lock().unwrap();
                    for path in paths {
                        watches.observe(
                            &self.root,
                            &path,
                            if result.get("bytes_written").is_some() {
                                "file_tool"
                            } else {
                                "before_file_tool"
                            },
                            cancellation,
                            &mut report,
                        );
                    }
                    watches.attach(&mut result, report);
                    Ok(result)
                }
                _ => Err(format!("Unknown tool: {name}")),
            }
        });
        let mut result =
            result.unwrap_or_else(|error| Value::object([("error", Value::string(error))]));
        let changes = self.protections.lock().unwrap().changes(
            &self.root,
            &self.outputs,
            cancellation,
            name == "protect",
        );
        if (!changes.is_empty() || name == "protect")
            && let Value::Object(fields) = &mut result
        {
            fields.insert("file_protections".into(), Value::Array(changes));
            if name == "protect" {
                fields.insert("file_protections_scope".into(), Value::string("all"));
            }
        }
        result
    }

    fn observe_file(
        &self,
        root: &Path,
        arguments: &Value,
        source: &str,
        cancellation: &Cancellation,
        result: &mut Value,
    ) {
        if root != self.root {
            if let Value::Object(fields) = result {
                fields.insert("file_scope".into(), Value::string("temporary"));
            }
            return;
        }
        let path = arguments.get("path").and_then(Value::as_str).unwrap_or("");
        if let Ok(path) = files::existing_path(root, path) {
            let mut report = watch::Report::default();
            let mut watches = self.watches.lock().unwrap();
            watches.observe(root, &path, source, cancellation, &mut report);
            watches.attach(result, report);
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
mod tests;
