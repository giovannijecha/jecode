use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

pub(super) fn remove(shell: &Shell, args: &[String]) -> Outcome {
    let recursive = args
        .iter()
        .any(|arg| arg == "-r" || arg == "-R" || arg == "-rf" || arg == "-fr");
    let force = args
        .iter()
        .any(|arg| arg == "-f" || arg == "-rf" || arg == "-fr");
    let paths: Vec<_> = args[1..]
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    if paths.is_empty() {
        return Err("rm: missing operand".into());
    }
    for path in paths {
        let full = shell.path(path);
        let result = if full.is_dir() {
            if recursive {
                fs::remove_dir_all(&full)
            } else {
                fs::remove_dir(&full)
            }
        } else {
            fs::remove_file(&full)
        };
        if let Err(error) = result
            && (!force || full.exists())
        {
            return Err(format!("rm: {path}: {error}"));
        }
    }
    ok(Vec::new())
}

pub(super) fn copy_move(shell: &Shell, args: &[String]) -> Outcome {
    let recursive = args.iter().any(|arg| arg == "-r" || arg == "-R");
    let operands: Vec<_> = args[1..]
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    if operands.len() != 2 {
        return Err(format!("{}: expected source and destination", args[0]));
    }
    let source = shell.path(operands[0]);
    let mut destination = shell.path(operands[1]);
    if destination.is_dir() {
        destination.push(source.file_name().ok_or("invalid source")?);
    }
    if source.is_dir() && inside(&source, &destination) {
        let action = if args[0] == "mv" { "move" } else { "copy" };
        return Err(format!(
            "{}: cannot {action} '{}' into itself",
            args[0], operands[0]
        ));
    }
    if args[0] == "mv" {
        if let Err(error) = fs::rename(&source, &destination) {
            if error.kind() != io::ErrorKind::CrossesDevices {
                return Err(format!("mv: {error}"));
            }
            copy_then_remove(&source, &destination).map_err(|e| format!("mv: {e}"))?;
        }
    } else if source.is_dir() {
        if !recursive {
            return Err("cp: directory requires -r".into());
        }
        copy_tree(&source, &destination).map_err(|e| format!("cp: {e}"))?;
    } else {
        fs::copy(&source, &destination).map_err(|e| format!("cp: {e}"))?;
    }
    ok(Vec::new())
}

/// Moves across volumes, where a rename cannot.
pub(in crate::shell) fn copy_then_remove(source: &Path, destination: &Path) -> io::Result<()> {
    if source.is_dir() {
        copy_tree(source, destination)?;
        fs::remove_dir_all(source)
    } else {
        fs::copy(source, destination)?;
        fs::remove_file(source)
    }
}

/// Whether `destination`, which may not exist yet, lies within `source`.
fn inside(source: &Path, destination: &Path) -> bool {
    fn resolve(path: &Path) -> Option<PathBuf> {
        match path.canonicalize() {
            Ok(path) => Some(path),
            Err(_) => Some(resolve(path.parent()?)?.join(path.file_name()?)),
        }
    }
    match (source.canonicalize(), resolve(destination)) {
        (Ok(source), Some(destination)) => destination.starts_with(source),
        _ => false,
    }
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

pub(super) fn mktemp(shell: &Shell, args: &[String]) -> Outcome {
    let directory = args.iter().any(|arg| arg == "-d");
    let templates: Vec<_> = args[1..].iter().filter(|arg| *arg != "-d").collect();
    if templates.len() > 1 {
        return Err("mktemp: expected one template".into());
    }
    let template = if let Some(template) = templates.first() {
        (*template).clone()
    } else {
        let base = shell.variables.get("TMPDIR").unwrap_or(".");
        format!("{base}/jecode.XXXXXXXXXX")
    };
    let x_count = template.chars().rev().take_while(|ch| *ch == 'X').count();
    if !(6..=64).contains(&x_count) {
        return Err("mktemp: template must end in 6 to 64 X characters".into());
    }
    let prefix = &template[..template.len() - x_count];
    for _ in 0..100 {
        let nonce = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let suffix = format!(
            "{:0width$x}",
            (std::process::id() as u64) << 32 | nonce,
            width = x_count
        );
        let path = shell.path(&format!("{prefix}{suffix}"));
        let result = if directory {
            fs::create_dir(&path)
        } else {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map(|_| ())
        };
        match result {
            Ok(()) => return ok(format!("{}\n", path.display()).into_bytes()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("mktemp: {}: {error}", path.display())),
        }
    }
    Err("mktemp: could not allocate a unique name".into())
}
