mod attachment;
mod files;
mod schema;
mod shell;

pub(crate) use files::read_text_page;
pub use schema::definitions;
pub use shell::find_bash;

use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) const OUTPUT_LIMIT: usize = 64 * 1024;

pub struct Tools {
    root: PathBuf,
    bash: PathBuf,
    outputs: crate::output::Store,
    temporary: Option<crate::scratch::Area>,
    attachments: Option<crate::attachments::Pool>,
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
                    if attachment::is_reference(path) {
                        attachment::read(self.attachments.as_ref(), path, &arguments, cancellation)
                    } else if crate::output::is_reference(path) {
                        files::read_page(&self.outputs.resolve(path)?, &arguments, cancellation)
                    } else {
                        let (root, arguments) = self.file_arguments(&arguments, false)?;
                        files::read(&root, &arguments, cancellation)
                    }
                }
                "write" | "edit" => {
                    let (root, arguments) = self.file_arguments(&arguments, name == "write")?;
                    if name == "write" {
                        files::write(&root, &arguments)
                    } else {
                        files::edit(&root, &arguments)
                    }
                }
                "bash" => {
                    let temporary = self
                        .temporary
                        .as_ref()
                        .map(|area| area.ensure())
                        .transpose()?;
                    Ok(shell::execute(
                        &self.bash,
                        &self.root,
                        &arguments,
                        cancellation,
                        &self.outputs,
                        temporary.as_deref(),
                    )
                    .unwrap_or_else(|error| Value::object([("error", Value::string(error))])))
                }
                _ => Err(format!("Unknown tool: {name}")),
            }
        });
        result.unwrap_or_else(|error| Value::object([("error", Value::string(error))]))
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
