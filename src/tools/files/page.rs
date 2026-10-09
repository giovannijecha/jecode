use crate::{
    cancel::Cancellation,
    json::Value,
    tools::{OUTPUT_LIMIT, optional_number},
};
use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;

pub(crate) fn read_page(
    path: &Path,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let file = File::open(path).map_err(|error| format!("Could not open file: {error}"))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("The path must refer to a regular file".into());
    }
    read_source(
        BufReader::new(file),
        metadata.len(),
        arguments,
        cancellation,
    )
}

pub(crate) fn read_text_page(
    text: &str,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    read_source(
        BufReader::new(Cursor::new(text.as_bytes())),
        text.len() as u64,
        arguments,
        cancellation,
    )
}

fn read_source(
    mut reader: impl BufRead + Seek,
    length: u64,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let offset = optional_number(arguments, "offset", 1, 1, usize::MAX)?;
    let limit = optional_number(arguments, "limit", 200, 1, 2000)?;
    let byte_mode = arguments.get("byte_offset").is_some();
    if let Some(byte_offset) = arguments.get("byte_offset") {
        let byte_offset = byte_offset
            .as_usize()
            .ok_or("byte_offset must be a nonnegative integer")? as u64;
        if byte_offset > length {
            return Err("byte_offset exceeds the saved file length".into());
        }
        reader
            .seek(SeekFrom::Start(byte_offset))
            .map_err(|error| error.to_string())?;
    } else {
        for _ in 1..offset {
            if cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            if reader
                .skip_until(b'\n')
                .map_err(|error| error.to_string())?
                == 0
            {
                return Err("Offset exceeds the file's lines".into());
            }
        }
    }
    if cancellation.requested() {
        return Err("Operation cancelled".into());
    }
    let start = reader
        .stream_position()
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    reader
        .take((OUTPUT_LIMIT + 3) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read file: {error}"))?;
    let valid = match std::str::from_utf8(&bytes) {
        Ok(text) => text.len(),
        Err(error) if error.error_len().is_none() && start + (bytes.len() as u64) < length => {
            error.valid_up_to()
        }
        Err(_) => return Err("File page is not UTF-8 text".into()),
    };
    let text = std::str::from_utf8(&bytes[..valid]).unwrap();
    if text.contains('\0') {
        return Err("File contains binary data".into());
    }
    let mut output = String::new();
    let mut consumed = 0;
    let mut shown = 0;
    let mut next_line = offset;
    for line in text.split_inclusive('\n').take(limit) {
        let prefix = if byte_mode {
            String::new()
        } else {
            format!("{next_line}: ")
        };
        let mut keep = line
            .len()
            .min(OUTPUT_LIMIT.saturating_sub(output.len() + prefix.len()));
        while !line.is_char_boundary(keep) {
            keep -= 1;
        }
        if keep == 0 {
            break;
        }
        output.push_str(&prefix);
        output.push_str(&line[..keep]);
        consumed += keep;
        shown += 1;
        if line[..keep].ends_with('\n') {
            next_line += 1;
        }
        if keep < line.len() {
            break;
        }
    }
    let next_byte = start + consumed as u64;
    let more = next_byte < length;
    let partial_line = more && consumed > 0 && bytes[consumed - 1] != b'\n';
    Ok(Value::object([
        ("content", Value::string(output)),
        (
            "total_lines",
            if more || byte_mode {
                Value::Null
            } else {
                Value::number(offset.saturating_sub(1) + shown)
            },
        ),
        (
            "offset",
            if byte_mode {
                Value::Null
            } else {
                Value::number(offset)
            },
        ),
        ("byte_offset", Value::number(start)),
        ("bytes_returned", Value::number(consumed)),
        ("lines_returned", Value::number(shown)),
        ("output_limit_bytes", Value::number(OUTPUT_LIMIT)),
        (
            "next_offset",
            if more && !byte_mode {
                Value::number(next_line)
            } else {
                Value::Null
            },
        ),
        (
            "next_byte_offset",
            if more {
                Value::number(next_byte)
            } else {
                Value::Null
            },
        ),
        ("partial_line", Value::Bool(partial_line)),
        ("truncated", Value::Bool(more)),
    ]))
}
