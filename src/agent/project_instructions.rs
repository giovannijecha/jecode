use super::Agent;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "AGENTS.md";
const BYTE_LIMIT: usize = 64 * 1024;

impl Agent {
    pub(super) fn refresh_project_instructions(&mut self) -> Result<(), String> {
        // Keep the source snapshot until the turn ends. Comparing before
        // redaction also catches edits between two differently masked secrets.
        let instructions = load(self.tools.root())?;
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
    let mut sections = Vec::new();
    let mut total = 0;
    scan(root, root, &mut sections, &mut total)?;
    Ok(sections.join("\n\n"))
}

fn scan(
    root: &Path,
    directory: &Path,
    sections: &mut Vec<String>,
    total: &mut usize,
) -> Result<(), String> {
    let relative = directory
        .strip_prefix(root)
        .expect("directory is below root");
    let file = directory.join(FILE_NAME);
    if let Some(text) = read_instruction(root, &file)? {
        *total += text.len();
        if *total > BYTE_LIMIT {
            return Err(format!(
                "{FILE_NAME} instructions exceed the combined 64 KiB source-text limit (at {})",
                file.display()
            ));
        }
        let source = if relative.as_os_str().is_empty() {
            FILE_NAME.to_owned()
        } else {
            relative
                .join(FILE_NAME)
                .to_string_lossy()
                .replace('\\', "/")
        };
        let scope = if relative.as_os_str().is_empty() {
            "the entire project tree".to_owned()
        } else {
            format!(
                "{} and its descendants",
                relative.to_string_lossy().replace('\\', "/")
            )
        };
        sections.push(format!(
            "Source: {source}\nDirectory scope: {scope}\n\n{text}"
        ));
    }

    let mut children = Vec::<PathBuf>::new();
    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "Could not scan {} for {FILE_NAME}: {error}",
            directory.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Could not scan {} for {FILE_NAME}: {error}",
                directory.display()
            )
        })?;
        if entry.file_name() == ".git" {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Could not inspect {}: {error}", entry.path().display()))?;
        if !file_type.is_dir() {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("Could not inspect {}: {error}", entry.path().display()))?;
        if metadata.is_dir() && !crate::disk::linked(&metadata) {
            children.push(entry.path());
        }
    }
    children.sort();
    for child in children {
        scan(root, &child, sections, total)?;
    }
    Ok(())
}

fn read_instruction(root: &Path, path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not inspect {}: {error}", path.display())),
    }
    let resolved = fs::canonicalize(path)
        .map_err(|error| format!("Could not resolve {}: {error}", path.display()))?;
    if !resolved.starts_with(root) {
        return Err(format!(
            "{} must resolve inside the working directory",
            path.display()
        ));
    }
    let metadata = fs::metadata(&resolved)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "{} must be a regular UTF-8 text file",
            path.display()
        ));
    }
    if metadata.len() > BYTE_LIMIT as u64 {
        return Err(format!(
            "{} exceeds the 64 KiB instruction limit",
            path.display()
        ));
    }
    let file = fs::File::open(&resolved)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(BYTE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    if bytes.len() > BYTE_LIMIT {
        return Err(format!(
            "{} exceeds the 64 KiB instruction limit",
            path.display()
        ));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("{} must contain valid UTF-8 text", path.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    Ok((!text.trim().is_empty()).then(|| text.to_owned()))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod link_tests;
