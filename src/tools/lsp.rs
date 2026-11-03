//! Provider-neutral access to explicitly enabled language-server extensions.

use super::Tools;
use crate::{
    cancel::Cancellation,
    json::Value,
    lsp::{Profile, Request, Runtime, Store},
    redact::Redactor,
};
use std::path::{Path, PathBuf};

mod bounded;
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;

impl Tools {
    pub fn configure_lsp(&mut self, home: &Path, redactor: Redactor) -> Result<(), String> {
        if self.lsp_home.as_deref() != Some(home) {
            self.lsp_runtime.shutdown();
            self.lsp_runtime = Runtime::with_home(home.to_path_buf());
        }
        self.lsp_home = Some(home.to_path_buf());
        self.lsp_redactor = redactor;
        self.lsp_runtime
            .include_credentials(self.lsp_redactor.clone());
        self.lsp_profiles().map(|_| ())
    }

    pub fn lsp_runtime(&self) -> Runtime {
        self.lsp_runtime.clone()
    }

    pub fn invalidate_analysis(&self) {
        self.lsp_runtime.invalidate_paths(&[]);
    }

    pub fn lsp_label(&self, arguments: &Value) -> Option<String> {
        let profiles = self.lsp_profiles().ok()?;
        if let Some(id) = arguments.get("server").and_then(Value::as_str) {
            profiles
                .iter()
                .find(|profile| profile.id == id)
                .map(|profile| profile.name.clone())
        } else {
            let path = self.root.join(arguments.get("path")?.as_str()?);
            let mut matching = profiles
                .iter()
                .filter(|profile| profile.enabled && profile.matches(&path));
            let profile = matching.next()?;
            matching.next().is_none().then(|| profile.name.clone())
        }
    }

    fn lsp_profiles(&self) -> Result<Vec<Profile>, String> {
        let home = self
            .lsp_home
            .as_ref()
            .ok_or("LSP configuration is unavailable without a user configuration directory")?;
        let profiles = Store::new(home.clone()).load()?;
        for profile in &profiles {
            for secret in profile.environment.values() {
                self.lsp_redactor.add_secret(secret.clone());
            }
        }
        Ok(profiles)
    }

    pub(super) fn lsp(&self, arguments: &Value, cancel: &Cancellation) -> Value {
        let run = || -> Result<Value, String> {
            let profiles = self.lsp_profiles()?;
            let request = request(arguments)?;
            let outcome: crate::lsp::Outcome = self
                .lsp_runtime
                .execute(&self.root, &profiles, request, cancel);
            let mut value = outcome.to_value();
            if let Value::Object(fields) = &mut value {
                if let Some(action) = arguments.get("action") {
                    fields.insert("action".into(), action.clone());
                }
                if let Some(profile) = outcome
                    .server
                    .as_deref()
                    .and_then(|id| profiles.iter().find(|profile| profile.id == id))
                {
                    fields.insert("server_name".into(), Value::string(&profile.name));
                }
                if let Some(Value::Array(servers)) = fields.get_mut("servers") {
                    for server in servers {
                        let profile = server
                            .get("id")
                            .and_then(Value::as_str)
                            .and_then(|id| profiles.iter().find(|profile| profile.id == id));
                        if let (Some(profile), Value::Object(fields)) = (profile, server) {
                            fields.insert("name".into(), Value::string(&profile.name));
                            fields.insert("enabled".into(), Value::Bool(profile.enabled));
                            fields.insert(
                                "languages".into(),
                                Value::Array(profile.languages.iter().map(Value::string).collect()),
                            );
                            fields.insert(
                                "extensions".into(),
                                Value::Object(
                                    profile
                                        .extensions
                                        .iter()
                                        .map(|(extension, language)| {
                                            (extension.clone(), Value::string(language))
                                        })
                                        .collect(),
                                ),
                            );
                        }
                    }
                }
            }
            Ok(value)
        };
        let value = run().unwrap_or_else(|error| {
            Value::object([
                ("status", Value::string("failed")),
                ("error", Value::string(error)),
                ("complete", Value::Bool(false)),
            ])
        });
        bounded::bound(self.lsp_redactor.value(&value), &self.outputs)
    }
}

fn request(arguments: &Value) -> Result<Request, String> {
    let server = optional_text(arguments, "server")?.map(str::to_owned);
    let action = super::required_string(arguments, "action")?;
    if action == "status" {
        return Ok(Request::Status { server });
    }
    let path = PathBuf::from(super::required_string(arguments, "path")?);
    if action == "diagnostics" {
        return Ok(Request::Diagnostics { path, server });
    }
    let position = |name| {
        arguments
            .get(name)
            .and_then(Value::as_usize)
            .filter(|value| (1..=i32::MAX as usize).contains(value))
            .map(|value| value as u32)
            .ok_or_else(|| format!("{name} must be a one-based positive integer"))
    };
    let (line, column) = (position("line")?, position("column")?);
    match action {
        "definition" => Ok(Request::Definition {
            path,
            line,
            column,
            server,
        }),
        "references" => {
            let include_declaration = match arguments.get("include_declaration") {
                Some(Value::Bool(value)) => *value,
                None => false,
                _ => return Err("include_declaration must be a boolean".into()),
            };
            Ok(Request::References {
                path,
                line,
                column,
                include_declaration,
                server,
            })
        }
        _ => Err("LSP action must be status, definition, references or diagnostics".into()),
    }
}

fn optional_text<'a>(value: &'a Value, name: &str) -> Result<Option<&'a str>, String> {
    match value.get(name) {
        None => Ok(None),
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(Some(text)),
        _ => Err(format!("{name} must be a nonempty string")),
    }
}
