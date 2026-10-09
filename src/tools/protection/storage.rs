use super::super::{files, watch::fingerprint};
use crate::{cancel::Cancellation, json::Value};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

pub(super) fn token(version: &Value) -> &str {
    version
        .get("fingerprint")
        .and_then(Value::as_str)
        .unwrap_or("missing")
}

fn copy(source: &Path, target: &Path, cancellation: &Cancellation) -> Result<u64, String> {
    let mut input = File::open(source).map_err(|error| error.to_string())?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(target).map_err(|error| error.to_string())?;
    let mut buffer = [0; 64 * 1024];
    let mut total = 0;
    let result = (|| {
        loop {
            if cancellation.requested() {
                return Err("Protection copy was cancelled".into());
            }
            let count = input.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|error| error.to_string())?;
            total += count as u64;
        }
        output.sync_all().map_err(|error| error.to_string())?;
        Ok(total)
    })();
    drop(output);
    if result.is_err() {
        let _ = fs::remove_file(target);
    }
    result
}

pub(super) fn snapshot(
    root: &Path,
    path: &Path,
    saved: &Path,
    cancellation: &Cancellation,
    expected: &Value,
) -> Result<Value, String> {
    let before = fingerprint::capture(root, path, cancellation)?;
    if &before != expected {
        return Err("File changed after its preservation intent was recorded".into());
    }
    copy(&root.join(path), saved, cancellation)?;
    let result = (|| {
        if fingerprint::capture(root, path, cancellation)? != before
            || fingerprint::capture(
                saved.parent().unwrap(),
                Path::new(saved.file_name().unwrap()),
                cancellation,
            )? != before
            || !same_bytes(&root.join(path), saved, cancellation)?
        {
            return Err("File changed while its protection baseline was being saved".into());
        }
        Ok(before)
    })();
    if result.is_err() {
        let _ = fs::remove_file(saved);
    }
    result
}

pub(super) fn same_bytes(
    left: &Path,
    right: &Path,
    cancellation: &Cancellation,
) -> Result<bool, String> {
    let mut left = File::open(left).map_err(|error| error.to_string())?;
    let mut right = File::open(right).map_err(|error| error.to_string())?;
    let before = left.metadata().map_err(|error| error.to_string())?;
    let right_metadata = right.metadata().map_err(|error| error.to_string())?;
    if !before.is_file() || !right_metadata.is_file() {
        return Err("Protection requires regular files".into());
    }
    if before.len() != right_metadata.len() {
        return Ok(false);
    }
    let mut a = [0; 64 * 1024];
    let mut b = [0; 64 * 1024];
    let mut remaining = before.len();
    while remaining > 0 {
        if cancellation.requested() {
            return Err("Protection comparison was cancelled".into());
        }
        let count = remaining.min(a.len() as u64) as usize;
        left.read_exact(&mut a[..count])
            .map_err(|error| error.to_string())?;
        right
            .read_exact(&mut b[..count])
            .map_err(|error| error.to_string())?;
        if a[..count] != b[..count] {
            return Ok(false);
        }
        remaining -= count as u64;
    }
    let after = left.metadata().map_err(|error| error.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("File changed during protection comparison".into());
    }
    Ok(true)
}

pub(super) fn verify_snapshot(
    saved: &Path,
    version: &Value,
    cancellation: &Cancellation,
) -> Result<(), String> {
    let captured = fingerprint::capture(
        saved.parent().unwrap(),
        Path::new(saved.file_name().unwrap()),
        cancellation,
    )?;
    if &captured != version {
        return Err("Protection baseline is missing or changed; it was not recreated".into());
    }
    Ok(())
}

pub(super) fn restore(
    root: &Path,
    path: &Path,
    saved: &Path,
    version: &Value,
    expected: &str,
    cancellation: &Cancellation,
) -> Result<u64, String> {
    verify_snapshot(saved, version, cancellation)?;
    let current = fingerprint::capture(root, path, cancellation)?;
    if token(&current) != expected {
        return Err(
            "Protected file changed since inspection; request protect status before restoring"
                .into(),
        );
    }
    let target = files::writable_path(root, &path.to_string_lossy())?;
    if target != root.join(path)
        || fs::symlink_metadata(root.join(path))
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(
            "Protected path was redirected through a link; review it before restoration".into(),
        );
    }
    let permissions = match fs::metadata(&target) {
        Ok(metadata) if metadata.permissions().readonly() => {
            return Err("Protected file is read-only; it was left unchanged".into());
        }
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    let temporary = target.with_file_name(format!(
        ".jecode-restore-{}.tmp",
        crate::sessions::identifier()
    ));
    let bytes = copy(saved, &temporary, cancellation)?;
    let result = (|| {
        if let Some(permissions) = permissions {
            fs::set_permissions(&temporary, permissions).map_err(|error| error.to_string())?;
        }
        verify_snapshot(saved, version, cancellation)?;
        if fingerprint::capture(root, path, cancellation)? != current {
            return Err(
                "Protected file changed while restoration was prepared; it was left unchanged"
                    .into(),
            );
        }
        if cancellation.requested() {
            return Err("Protection restoration was cancelled".into());
        }
        fs::rename(&temporary, &target).map_err(|error| error.to_string())?;
        Ok(bytes)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
