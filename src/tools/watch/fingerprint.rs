use crate::cancel::Cancellation;
use crate::json::Value;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::Hasher;
use std::io::Read;
use std::path::Path;

pub(in crate::tools) fn capture(
    root: &Path,
    path: &Path,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let absolute = root.join(path);
    let resolved = match fs::canonicalize(&absolute) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(&absolute)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(
                    "The observed symbolic link no longer resolves to a scoped file".into(),
                );
            }
            // A missing file is observable only while its nearest existing parent stays scoped.
            let mut parent = absolute.parent();
            while let Some(candidate) = parent {
                match fs::canonicalize(candidate) {
                    Ok(parent) if parent.starts_with(root) => {
                        return Ok(Value::object([("state", Value::string("missing"))]));
                    }
                    Ok(_) => return Err("Path now resolves outside the working directory".into()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        parent = candidate.parent();
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            return Err("Could not resolve the file's parent".into());
        }
        Err(error) => return Err(error.to_string()),
    };
    if !resolved.starts_with(root) {
        return Err("Path now resolves outside the working directory".into());
    }
    let mut file = File::open(&resolved).map_err(|error| error.to_string())?;
    let before = file.metadata().map_err(|error| error.to_string())?;
    if !before.is_file() {
        return Err("The observed path is no longer a regular file".into());
    }
    let mut hash = DefaultHasher::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        if cancellation.requested() {
            return Err("File comparison was cancelled".into());
        }
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hash.write(&buffer[..count]);
        bytes += count as u64;
    }
    let after = fs::metadata(&absolute).map_err(|error| error.to_string())?;
    if bytes != before.len()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || fs::canonicalize(&absolute).map_err(|error| error.to_string())? != resolved
    {
        return Err("File changed while its content was being compared".into());
    }
    Ok(Value::object([
        ("state", Value::string("present")),
        ("bytes", Value::number(bytes)),
        (
            "fingerprint",
            Value::string(format!("rust-default:{:016x}", hash.finish())),
        ),
    ]))
}

pub(in crate::tools) fn version(value: &Value) -> Option<Value> {
    match value.get("state").and_then(Value::as_str) {
        Some("missing") => Some(Value::object([("state", Value::string("missing"))])),
        Some("present") => Some(Value::object([
            ("state", Value::string("present")),
            ("bytes", value.get("bytes")?.as_usize().map(Value::number)?),
            (
                "fingerprint",
                Value::string(value.get("fingerprint")?.as_str()?),
            ),
        ])),
        _ => None,
    }
}
