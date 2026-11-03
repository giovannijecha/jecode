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
        let mut instructions = load(self.tools.root())?;
        if let Some(inherited) = &self.inherited_project_instructions
            && !inherited.is_empty()
        {
            instructions = if instructions.is_empty() {
                inherited.clone()
            } else if instructions == *inherited {
                instructions
            } else {
                format!("{inherited}\n\n{instructions}")
            };
        }
        if instructions != self.project_instructions {
            self.project_instructions = instructions;
            // The measured prompt contained a different system projection.
            // Keep its conservative density and learned context ceiling.
            self.context.clear_measurement();
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
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "Could not scan {} for {FILE_NAME}: {error}",
                directory.display()
            ));
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "Could not scan {} for {FILE_NAME}: {error}",
                    directory.display()
                ));
            }
        };
        if matches!(
            entry.file_name().to_str(),
            Some(".git" | ".tmp" | "node_modules" | ".venv" | "vendor")
        ) {
            // Working files can contain temporary checkouts with unrelated
            // instructions, including files still being written or removed.
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "Could not inspect {}: {error}",
                    entry.path().display()
                ));
            }
        };
        if !file_type.is_dir() {
            continue;
        }
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "Could not inspect {}: {error}",
                    entry.path().display()
                ));
            }
        };
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
    let resolved = match fs::canonicalize(path) {
        Ok(resolved) => resolved,
        Err(error) if error.kind() == ErrorKind::NotFound && vanished(path) => return Ok(None),
        Err(error) => return Err(format!("Could not resolve {}: {error}", path.display())),
    };
    if !resolved.starts_with(root) {
        return Err(format!(
            "{} must resolve inside the working directory",
            path.display()
        ));
    }
    let metadata = match fs::metadata(&resolved) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound && vanished(path) => return Ok(None),
        Err(error) => return Err(format!("Could not inspect {}: {error}", path.display())),
    };
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
    let file = match fs::File::open(&resolved) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound && vanished(path) => return Ok(None),
        Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
    };
    let mut bytes = Vec::new();
    match file.take(BYTE_LIMIT as u64 + 1).read_to_end(&mut bytes) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound && vanished(path) => return Ok(None),
        Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
    }
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

fn vanished(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path), Err(error) if error.kind() == ErrorKind::NotFound)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod link_tests;

#[cfg(test)]
mod dependency_tests;
