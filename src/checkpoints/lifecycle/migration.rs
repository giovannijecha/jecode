//! Crash-resumable layout upgrade for idle version 1 archives. Root CURRENT is
//! deliberately invalid to older binaries before any history is relocated.
use super::{generation_locks, old_owner_active, ordinary_directory};
use crate::checkpoints::{files, journal, layout, locks::Exclusive};
use crate::json::{self, Value};
use std::fs;
use std::path::Path;

const SENTINEL: &[u8] = b"migrated-to-history";

/// Called under the stable lifecycle and root store locks. `false` means an
/// old reader is still active, so neither migration nor expiry can proceed.
pub(super) fn resume_or_start(archive: &Path) -> Result<bool, String> {
    let marker = archive.join(".migrating");
    if layout::version(archive) == Some(2) {
        if marker.exists() {
            verify_completed(archive)?;
            fs::remove_file(marker)
                .map_err(|error| format!("Could not finish checkpoint migration: {error}"))?;
        }
        return Ok(true);
    }
    if layout::version(archive) != Some(1) {
        return Err("Checkpoint identity has an unsupported layout".into());
    }
    if old_owner_active(archive)? {
        return Ok(false);
    }
    for path in generation_locks(archive)? {
        if Exclusive::try_acquire(&path)
            .map_err(|error| format!("Could not inspect legacy checkpoint reader: {error}"))?
            .is_none()
        {
            return Ok(false);
        }
    }
    let (generation, had_current) = if marker.exists() {
        read_marker(&marker)?
    } else {
        let generation = journal::current_generation(archive)?;
        let had_current = archive.join("CURRENT").exists();
        files::atomic_write(
            &marker,
            format!("{generation} {}", u8::from(had_current)).as_bytes(),
        )
        .map_err(|error| format!("Could not start checkpoint migration: {error}"))?;
        (generation, had_current)
    };
    if fs::read(archive.join("CURRENT")).ok().as_deref() != Some(SENTINEL) {
        files::atomic_write(&archive.join("CURRENT"), SENTINEL)
            .map_err(|error| format!("Could not guard legacy checkpoint history: {error}"))?;
    }
    let history = archive.join("history");
    let runtime = archive.join("runtime");
    if history.exists() && !ordinary_directory(&history) {
        return Err("Checkpoint history directory is not an ordinary directory".into());
    }
    if runtime.exists() && !ordinary_directory(&runtime) {
        return Err("Checkpoint runtime directory is not an ordinary directory".into());
    }
    files::private_directory(&history)
        .map_err(|error| format!("Could not create checkpoint history directory: {error}"))?;
    files::private_directory(&runtime)
        .map_err(|error| format!("Could not create checkpoint runtime directory: {error}"))?;
    for entry in fs::read_dir(archive)
        .map_err(|error| format!("Could not inspect legacy checkpoint history: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("Could not inspect legacy checkpoint entry: {error}"))?;
        let name = entry.file_name();
        let Some(text) = name.to_str() else { continue };
        if generation_entry(text) {
            move_entry(&entry.path(), &history.join(&name))?;
        }
    }
    for name in ["owners", "leases", "activity"] {
        move_directory_contents(&archive.join(name), &runtime.join(name))?;
    }
    if had_current {
        files::atomic_write(&history.join("CURRENT"), generation.to_string().as_bytes()).map_err(
            |error| format!("Could not publish migrated checkpoint generation: {error}"),
        )?;
    }
    files::sync_directory(&history)
        .map_err(|error| format!("Could not flush migrated checkpoint history: {error}"))?;
    files::sync_directory(&runtime)
        .map_err(|error| format!("Could not flush migrated checkpoint runtime: {error}"))?;
    let identity = archive.join("workspace.json");
    let text = fs::read_to_string(&identity)
        .map_err(|error| format!("Could not read checkpoint identity: {error}"))?;
    let mut value = json::parse(&text)?;
    let Value::Object(ref mut fields) = value else {
        return Err("Checkpoint identity is damaged".into());
    };
    fields.insert("version".into(), Value::number(2));
    files::atomic_write(&identity, value.pretty().as_bytes())
        .map_err(|error| format!("Could not publish checkpoint migration: {error}"))?;
    fs::remove_file(marker)
        .map_err(|error| format!("Could not finish checkpoint migration: {error}"))?;
    files::sync_directory(archive)
        .map_err(|error| format!("Could not flush checkpoint migration: {error}"))?;
    Ok(true)
}

fn read_marker(path: &Path) -> Result<(u64, bool), String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Could not read checkpoint migration: {error}"))?;
    let mut parts = text.split_whitespace();
    let generation = parts.next().and_then(|part| part.parse().ok());
    let had_current = match parts.next() {
        Some("0") => Some(false),
        Some("1") => Some(true),
        _ => None,
    };
    match (generation, had_current, parts.next()) {
        (Some(generation), Some(had_current), None) => Ok((generation, had_current)),
        _ => Err("Checkpoint migration marker is damaged".into()),
    }
}

fn verify_completed(archive: &Path) -> Result<(), String> {
    if fs::read(archive.join("CURRENT")).ok().as_deref() != Some(SENTINEL)
        || !ordinary_directory(&archive.join("history"))
    {
        return Err("Checkpoint migration is incomplete; history was preserved".into());
    }
    Ok(())
}

fn generation_entry(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix("gen-") else {
        return false;
    };
    let suffix = suffix.strip_suffix(".lock").unwrap_or(suffix);
    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
}

fn move_entry(source: &Path, destination: &Path) -> Result<(), String> {
    if fs::symlink_metadata(source).map_or(true, |metadata| metadata.file_type().is_symlink()) {
        return Err(format!(
            "Checkpoint migration found an unsafe entry at {}",
            source.display()
        ));
    }
    if fs::symlink_metadata(destination).is_ok() {
        return Err(format!(
            "Checkpoint migration found duplicate history at {}; both copies were preserved",
            destination.display()
        ));
    }
    fs::rename(source, destination)
        .map_err(|error| format!("Could not move checkpoint history: {error}"))
}

fn move_directory_contents(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    if !ordinary_directory(source) || (destination.exists() && !ordinary_directory(destination)) {
        return Err("Checkpoint migration found an unsafe runtime directory".into());
    }
    files::private_directory(destination)
        .map_err(|error| format!("Could not prepare checkpoint runtime: {error}"))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("Could not inspect legacy checkpoint runtime: {error}"))?
    {
        let entry = entry
            .map_err(|error| format!("Could not inspect legacy checkpoint runtime: {error}"))?;
        move_entry(&entry.path(), &destination.join(entry.file_name()))?;
    }
    fs::remove_dir(source)
        .map_err(|error| format!("Could not finish checkpoint runtime migration: {error}"))?;
    Ok(())
}
