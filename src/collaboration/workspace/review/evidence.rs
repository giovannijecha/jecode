use super::{Entry, Mode, Workspace, files, inspect};
use crate::json::Value;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(super) fn evidence(workspace: &Workspace, entry: &Entry) -> Result<Value, String> {
    let regular = |state: &files::Fingerprint| {
        matches!(
            state,
            files::Fingerprint::File { .. } | files::Fingerprint::Absent
        )
    };
    if !regular(&entry.baseline) || !regular(&entry.child) {
        return Ok(Value::object([
            ("kind", Value::string("metadata")),
            (
                "detail",
                Value::string("Inspect directory or link state before selecting integration paths"),
            ),
        ]));
    }
    let max = crate::file_change::MAX_DIFF_BYTES as u64;
    if [&entry.baseline, &entry.child]
        .iter()
        .any(|state| matches!(state, files::Fingerprint::File { size, .. } if *size > max))
    {
        return Ok(Value::object([
            ("kind", Value::string("large_file")),
            (
                "detail",
                Value::string("Use diff with path and byte offset to inspect content"),
            ),
        ]));
    }
    let before = read_small(
        &workspace.storage.join("base"),
        &entry.path,
        &entry.baseline,
    )?;
    let after = read_small(&workspace.root, &entry.path, &entry.child)?;
    let (before, after) = match (before, after) {
        (Some(before), Some(after)) => (before, after),
        _ => {
            return Ok(Value::object([
                ("kind", Value::string("binary")),
                (
                    "detail",
                    Value::string(
                        "Binary content or invalid UTF-8; fingerprints give exact sizes and content identity",
                    ),
                ),
            ]));
        }
    };
    if let Some(change) = crate::file_change::Change::between(
        entry.baseline.exists().then_some(before.as_str()),
        &after,
    ) {
        let value = change.value();
        if value.encode().len() <= 16_384 {
            return Ok(Value::object([
                ("kind", Value::string("text_diff")),
                ("diff", value),
            ]));
        }
    }
    Ok(Value::object([
        ("kind", Value::string("text_preview")),
        (
            "before",
            Value::string(&before[..before.floor_char_boundary(2048)]),
        ),
        (
            "after",
            Value::string(&after[..after.floor_char_boundary(2048)]),
        ),
        (
            "detail",
            Value::string(
                "Diff is larger than the inline limit; use diff with path and byte offset",
            ),
        ),
    ]))
}

fn read_small(
    root: &Path,
    name: &str,
    state: &files::Fingerprint,
) -> Result<Option<String>, String> {
    if matches!(state, files::Fingerprint::Absent) {
        return Ok(Some(String::new()));
    }
    files::verify_content(root, name, state)?;
    let bytes = fs::read(files::path(root, name)?).map_err(|error| error.to_string())?;
    files::verify_content(root, name, state)?;
    Ok(String::from_utf8(bytes)
        .ok()
        .filter(|text| !text.contains('\0')))
}

pub(in crate::collaboration::workspace) fn inspect_file(
    workspace: &Workspace,
    path: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    if workspace.mode != Mode::Isolated {
        return Err("Shared work has no separate baseline".into());
    }
    if limit == 0 || limit > 65_536 {
        return Err("limit must be 1-65536 bytes".into());
    }
    files::validate_name(path)?;
    let review = inspect(workspace)?;
    let entry = review
        .entries
        .iter()
        .find(|entry| entry.path == path)
        .ok_or("Path is not changed in this workspace")?;
    let before = page(
        &workspace.storage.join("base"),
        path,
        &entry.baseline,
        offset,
        limit,
    )?;
    let after = page(&workspace.root, path, &entry.child, offset, limit)?;
    Ok(Value::object([
        ("path", Value::string(path)),
        ("offset", Value::number(offset)),
        ("before", before),
        ("after", after),
        ("review_token", Value::string(review.token)),
    ]))
}

fn page(
    root: &Path,
    name: &str,
    state: &files::Fingerprint,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    let size = match state {
        files::Fingerprint::File { size, .. } => *size,
        files::Fingerprint::Absent => {
            return Ok(Value::object([
                ("content", Value::string("")),
                ("size", Value::number(0)),
                ("next_offset", Value::Null),
            ]));
        }
        _ => return Err(format!("{name} is not a regular text file")),
    };
    if offset as u64 > size {
        return Err(format!("Offset exceeds {name} length"));
    }
    files::verify_content(root, name, state)?;
    let mut file = fs::File::open(files::path(root, name)?).map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(offset as u64))
        .map_err(|error| error.to_string())?;
    let mut bytes = vec![0u8; (size - offset as u64).min(limit as u64 + 3) as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    files::verify_content(root, name, state)?;
    let valid = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.len(),
        Err(error)
            if error.error_len().is_none() && offset as u64 + (bytes.len() as u64) < size =>
        {
            error.valid_up_to()
        }
        Err(_) => {
            return Err(format!(
                "{name} page is not UTF-8 text or offset splits a character"
            ));
        }
    };
    let text = std::str::from_utf8(&bytes[..valid]).map_err(|error| error.to_string())?;
    if text.contains('\0') {
        return Err(format!("{name} contains binary data"));
    }
    let keep = text.floor_char_boundary(limit);
    let next = offset + keep;
    Ok(Value::object([
        ("content", Value::string(&text[..keep])),
        ("size", Value::number(size)),
        (
            "next_offset",
            if (next as u64) < size {
                Value::number(next)
            } else {
                Value::Null
            },
        ),
    ]))
}
