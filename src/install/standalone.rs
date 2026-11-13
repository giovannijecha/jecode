//! Standalone installations from `INSTALL.ps1` or `INSTALL.sh`. Updates
//! come from the published npm release, read without Node or npm; uninstall
//! removes only the files the installer and updates create.

use super::provenance::{self, MARKER};
use super::registry::{self, Release};
use super::system;
use crate::json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const STAGE: &str = ".jecode-update";
const PREVIOUS: &str = ".jecode-previous";

pub struct Outcome {
    pub text: String,
    pub value: Value,
}

/// The release's executable for this platform, after the tarball's own
/// manifest confirms its version and SHA-256.
pub fn executable(tarball: &[u8], release: &Release) -> Result<Vec<u8>, String> {
    let tar = super::archive::gunzip(tarball, 512 * 1024 * 1024)?;
    let mut files = super::archive::untar(&tar)?;
    let (key, name) = provenance::target();
    let manifest = files
        .get("package/package.json")
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .and_then(|text| crate::json::parse(text).ok())
        .ok_or("The release has no readable package.json")?;
    if manifest.get("name").and_then(Value::as_str) != Some(registry::PACKAGE)
        || manifest.get("version").and_then(Value::as_str) != Some(&release.version)
    {
        return Err("The release package name or version does not match the registry".into());
    }
    let receipt = files
        .get("package/native/manifest.json")
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .and_then(|text| crate::json::parse(text).ok())
        .ok_or("The release has no native manifest")?;
    let artifact = receipt
        .get("artifacts")
        .and_then(Value::as_array)
        .and_then(|artifacts| {
            artifacts
                .iter()
                .find(|item| item.get("target").and_then(Value::as_str) == Some(key))
        })
        .ok_or_else(|| format!("The release has no {key} executable"))?;
    let binary = files
        .remove(&format!("package/native/{key}/{name}"))
        .ok_or_else(|| format!("The release has no {key} executable"))?;
    let expected = artifact.get("sha256").and_then(Value::as_str);
    if artifact.get("version").and_then(Value::as_str) != Some(&release.version)
        || expected != Some(crate::permissions::identity::sha256_hex(&binary)?.as_str())
    {
        return Err("The release executable does not match its manifest".into());
    }
    Ok(binary)
}

/// Runs `binary --version` and checks that it reports `version`.
pub fn validate(binary: &Path, version: &str) -> Result<(), String> {
    let mut child = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("The new executable could not start: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("The new executable did not answer --version".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !text.starts_with(&format!("jecode {version} (")) {
        return Err(format!(
            "The new executable reported {:?} instead of Jecode {version}",
            text.trim()
        ));
    }
    Ok(())
}

fn owned_leftover(name: &str) -> bool {
    let suffix = [STAGE, PREVIOUS].into_iter().find_map(|stem| {
        name.strip_prefix(stem)
            .and_then(|rest| rest.strip_prefix('-'))
    });
    let Some(suffix) = suffix else { return false };
    let suffix = if cfg!(windows) {
        match suffix.strip_suffix(".exe") {
            Some(suffix) => suffix,
            None => return false,
        }
    } else {
        suffix
    };
    suffix
        .split_once('-')
        .is_some_and(|(pid, millis)| pid.parse::<u32>().is_ok() && millis.parse::<u64>().is_ok())
}

fn leftovers(bin: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(bin)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    owned_leftover(&name) && entry.file_type().is_ok_and(|kind| kind.is_file())
                })
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default()
}

/// Stages `payload` beside `binary`, validates it, then swaps it in. Any
/// failure before the swap leaves the installation untouched; a failed swap
/// puts the previous executable back.
pub fn replace(
    binary: &Path,
    payload: &[u8],
    validate: impl Fn(&Path) -> Result<(), String>,
) -> Result<Vec<PathBuf>, String> {
    let bin = binary.parent().ok_or("The executable has no directory")?;
    for leftover in leftovers(bin) {
        let _ = std::fs::remove_file(leftover);
    }
    let extension = if cfg!(windows) { ".exe" } else { "" };
    let staged = bin.join(format!(
        "{STAGE}-{}-{}{extension}",
        std::process::id(),
        crate::sessions::now()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .map_err(|error| format!("Could not stage the update in {}: {error}", bin.display()))?;
        file.write_all(payload)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("Could not write the staged update: {error}"))?;
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
                .map_err(|error| error.to_string())?;
        }
        validate(&staged)?;
        activate(binary, &staged)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

/// Puts `staged` at `binary`. Returns earlier executables left to remove.
fn activate(binary: &Path, staged: &Path) -> Result<Vec<PathBuf>, String> {
    if cfg!(windows) {
        // A running program cannot be overwritten on Windows, but it can be
        // renamed; its name is then free for the new executable.
        let bin = binary.parent().unwrap();
        let previous = system::move_aside(binary, &[bin.to_path_buf()], PREVIOUS)?;
        if let Err(error) = std::fs::rename(staged, binary) {
            previous.restore()?;
            return Err(format!("Could not activate the update: {error}"));
        }
        Ok(vec![previous.to])
    } else {
        std::fs::rename(staged, binary)
            .map_err(|error| format!("Could not activate the update: {error}"))?;
        Ok(Vec::new())
    }
}

pub fn update(root: &Path, binary: &Path, release: &Release) -> Result<Outcome, String> {
    supported()?;
    eprintln!(
        "Downloading Jecode {} from {}…",
        release.version,
        registry::registry()
    );
    let tarball = registry::download(release)?;
    let payload = executable(&tarball, release)?;
    let previous = replace(binary, &payload, |staged| {
        validate(staged, &release.version)
    })?;
    let deferred = system::remove_later(&previous, &[]);
    let mut text = format!(
        "Updated {} to Jecode {} · sha512 and manifest verified",
        binary.display(),
        release.version
    );
    if !deferred.is_empty() {
        text.push_str(
            "\nThe previous executable is still running; it is removed after Jecode exits, or by the next update.",
        );
    }
    Ok(Outcome {
        text,
        value: Value::object([
            ("updated", Value::Bool(true)),
            ("version", Value::string(&release.version)),
            ("location", Value::string(root.to_string_lossy())),
            (
                "deferred",
                Value::Array(
                    deferred
                        .iter()
                        .map(|path| Value::string(path.to_string_lossy()))
                        .collect(),
                ),
            ),
        ]),
    })
}

pub fn supported() -> Result<(), String> {
    if !provenance::published() {
        return Err(format!(
            "Jecode releases include no executable for {}-{}; update this installation the way you built it",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    }
    Ok(())
}

/// Removes the executable, update leftovers, the marker and directories left
/// empty. Other files in the installation root and the Jecode home stay.
pub fn uninstall(root: &Path, binary: &Path, path_key: Option<&str>) -> Result<Outcome, String> {
    let bin = binary.parent().ok_or("The executable has no directory")?;
    let mut notes = Vec::new();
    let mut files = leftovers(bin);
    // Keep the executable restorable until the ownership marker is removed.
    // This also lets Windows remove the running executable after this process exits.
    let moved = system::move_aside(binary, &[bin.to_path_buf()], ".jecode-uninstalled")?;
    let marker = root.join(MARKER);
    if let Err(error) = std::fs::remove_file(&marker) {
        moved.restore().map_err(|restore| {
            format!("Could not remove {}: {error}; {restore}", marker.display())
        })?;
        return Err(format!("Could not remove {}: {error}", marker.display()));
    }
    files.push(moved.to.clone());
    let deferred = system::remove_later(&files, &[bin.to_path_buf(), root.to_path_buf()]);
    let kept: Vec<PathBuf> = [bin, root]
        .iter()
        .filter(|directory| directory.exists())
        .flat_map(|directory| {
            std::fs::read_dir(directory)
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .map(|entry| entry.path())
                        .filter(|path| !files.contains(path) && path != bin)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect();
    // A bin folder that still holds other programs stays on PATH for them.
    #[cfg(windows)]
    let (path_removed, path_noted) = match path_key {
        Some(_) if kept.iter().any(|path| path.parent() == Some(bin)) => {
            notes.push(format!(
                "Kept {} on your user PATH because it holds other files.",
                bin.display()
            ));
            (0, true)
        }
        Some(key) => match system::remove_user_path(bin, key) {
            Ok(count) => (count, false),
            Err(error) => {
                notes.push(format!("Could not remove the user PATH entry: {error}"));
                (0, true)
            }
        },
        None => (0, false),
    };
    #[cfg(not(windows))]
    let path_removed = {
        let _ = path_key;
        0
    };
    #[cfg(windows)]
    if !path_noted {
        notes.push(if path_removed > 0 {
            format!(
                "Removed {} from your user PATH; open a new terminal.",
                bin.display()
            )
        } else {
            "Your user PATH had no entry for it.".to_owned()
        });
    }
    #[cfg(not(windows))]
    {
        notes.push(format!(
            "If you added {} to PATH in a shell profile, remove that line.",
            bin.display()
        ));
    }
    if cfg!(windows) && !deferred.is_empty() {
        notes.push("The running executable is removed after Jecode exits.".into());
    }
    if !kept.is_empty() {
        notes.push(format!(
            "Kept files the installer did not create: {}",
            kept.iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(Outcome {
        text: format!(
            "Removed Jecode from {}\n{}",
            root.display(),
            notes.join("\n")
        ),
        value: Value::object([
            ("removed", Value::Bool(true)),
            ("location", Value::string(root.to_string_lossy())),
            ("path_entries_removed", Value::number(path_removed)),
            (
                "deferred",
                Value::Array(
                    deferred
                        .iter()
                        .map(|path| Value::string(path.to_string_lossy()))
                        .collect(),
                ),
            ),
            (
                "kept",
                Value::Array(
                    kept.iter()
                        .map(|path| Value::string(path.to_string_lossy()))
                        .collect(),
                ),
            ),
        ]),
    })
}

#[cfg(test)]
mod tests;
