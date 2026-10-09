use super::Agent;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::Path;

const FILE_NAME: &str = "JECODE.md";
const BYTE_LIMIT: usize = 64 * 1024;

impl Agent {
    pub(super) fn refresh_project_instructions(&mut self) -> Result<(), String> {
        let instructions = self.redact(&load(self.tools.root())?);
        if instructions != self.project_instructions {
            self.project_instructions = instructions;
            // The measured prompt contained a different system projection.
            // Keep its conservative density and learned context ceiling.
            self.context.input_tokens = None;
            self.context.measured_end = 0;
        }
        Ok(())
    }
}

fn load(root: &Path) -> Result<String, String> {
    let path = root.join(FILE_NAME);
    match fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(format!("Could not inspect {FILE_NAME}: {error}")),
    }
    let path = fs::canonicalize(path)
        .map_err(|error| format!("Could not resolve {FILE_NAME}: {error}"))?;
    if !path.starts_with(root) {
        return Err(format!(
            "{FILE_NAME} must resolve inside the working directory"
        ));
    }
    let metadata =
        fs::metadata(&path).map_err(|error| format!("Could not inspect {FILE_NAME}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("{FILE_NAME} must be a regular UTF-8 text file"));
    }
    if metadata.len() > BYTE_LIMIT as u64 {
        return Err(format!("{FILE_NAME} exceeds the 64 KiB instruction limit"));
    }
    let file =
        fs::File::open(path).map_err(|error| format!("Could not read {FILE_NAME}: {error}"))?;
    let mut bytes = Vec::new();
    file.take(BYTE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read {FILE_NAME}: {error}"))?;
    if bytes.len() > BYTE_LIMIT {
        return Err(format!("{FILE_NAME} exceeds the 64 KiB instruction limit"));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("{FILE_NAME} must contain valid UTF-8 text"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    Ok(if text.trim().is_empty() {
        String::new()
    } else {
        text.to_owned()
    })
}

#[cfg(test)]
mod tests;
