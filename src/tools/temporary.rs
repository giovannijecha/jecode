//! Reads of tmp: files across one conversation's agents. A file missing from
//! the agent's own area is read from the agents above it or, for a main agent,
//! from its collaborators; `tmp:<agent>:path` names one collaborator. Writes
//! and edits stay in the agent's own area.

use super::Tools;
use crate::json::Value;
use std::path::{Path, PathBuf};

impl Tools {
    pub(super) fn read_arguments(&self, arguments: &Value) -> Result<(PathBuf, Value), String> {
        let alias = arguments
            .get("path")
            .and_then(Value::as_str)
            .and_then(|path| path.strip_prefix("tmp:"));
        if let Some((agent, file)) = alias.and_then(collaborator) {
            let (_, root, path) = self
                .collaborators
                .temporary_files(Some(agent), file)?
                .into_iter()
                .next()
                .ok_or_else(|| format!("No agent {agent} of this conversation has tmp:{file}"))?;
            return Ok((root, located(arguments, &path)));
        }
        let error = match self.file_arguments(arguments, false) {
            Err(error) => error,
            found => return found,
        };
        let Some(alias) = alias else {
            return Err(error);
        };
        if let Some((root, path)) = self
            .inherited_temporary
            .iter()
            .find_map(|area| area.existing(alias))
        {
            return Ok((root, located(arguments, &path)));
        }
        match self.collaborators.temporary_files(None, alias)?.as_slice() {
            [] => Err(error),
            [(_, root, path)] => Ok((root.clone(), located(arguments, path))),
            found => {
                let agents: Vec<_> = found.iter().map(|(agent, ..)| agent.as_str()).collect();
                Err(format!(
                    "tmp:{alias} is not in your temporary area but in those of agents {}; read one as tmp:{}:{alias}",
                    agents.join(", "),
                    agents[0]
                ))
            }
        }
    }
}

/// Writes and edits never reach a collaborator's area.
pub(super) fn own(path: &str) -> Result<(), String> {
    match path.strip_prefix("tmp:").and_then(collaborator) {
        Some((_, file)) => Err(format!(
            "{path} is a collaborator's temporary file and read-only; write tmp:{file} in your own area"
        )),
        None => Ok(()),
    }
}

/// The collaborator and its file named by a `tmp:<agent>:path` alias.
fn collaborator(alias: &str) -> Option<(&str, &str)> {
    let (agent, file) = alias.split_once(':')?;
    let number = agent.strip_prefix('a')?;
    (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some((agent, file))
}

fn located(arguments: &Value, path: &Path) -> Value {
    let mut arguments = arguments.clone();
    if let Value::Object(fields) = &mut arguments {
        fields.insert("path".into(), Value::string(path.to_string_lossy()));
    }
    arguments
}
