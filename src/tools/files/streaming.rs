//! Exact large-text edits with bounded working memory and a staged replacement.
use super::*;
use std::io::{Seek, SeekFrom};

const BLOCK: usize = 64 * 1024;

struct Temporary(PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Streaming KMP includes overlapping occurrences and matches across reads.
fn locate(
    mut source: impl Read,
    pattern: &[u8],
    cancellation: &Cancellation,
) -> Result<(Option<u64>, bool), String> {
    let mut failure = vec![0; pattern.len()];
    for index in 1..pattern.len() {
        let mut matched = failure[index - 1];
        while matched > 0 && pattern[index] != pattern[matched] {
            matched = failure[matched - 1];
        }
        if pattern[index] == pattern[matched] {
            matched += 1;
        }
        failure[index] = matched;
    }
    let mut text = crate::utf8::Text::default();
    let mut buffer = [0; BLOCK];
    let mut matched = 0;
    let mut offset = 0u64;
    let mut position = None;
    let mut ambiguous = false;
    loop {
        check_cancelled(cancellation)?;
        let count = source
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        text.push(&buffer[..count])?;
        for byte in &buffer[..count] {
            while matched > 0 && *byte != pattern[matched] {
                matched = failure[matched - 1];
            }
            if *byte == pattern[matched] {
                matched += 1;
            }
            offset += 1;
            if matched == pattern.len() {
                let found = offset - pattern.len() as u64;
                if position.is_some() {
                    ambiguous = true;
                } else {
                    position = Some(found);
                }
                matched = failure[matched - 1];
            }
        }
    }
    text.finish()?;
    Ok((position, ambiguous))
}

fn mismatch(source: &mut fs::File, length: u64, error: &str) -> Result<Value, String> {
    source.rewind().map_err(|error| error.to_string())?;
    let mut context = Vec::new();
    source
        .take(8192)
        .read_to_end(&mut context)
        .map_err(|error| error.to_string())?;
    let valid = match std::str::from_utf8(&context) {
        Ok(_) => context.len(),
        Err(error) => error.valid_up_to(),
    };
    let mut result =
        super::mismatch::result(std::str::from_utf8(&context[..valid]).unwrap(), error);
    if let Value::Object(fields) = &mut result {
        fields.insert("current_bytes".into(), Value::number(length));
        fields.insert(
            "context_complete".into(),
            Value::Bool(valid as u64 == length),
        );
    }
    Ok(result)
}

fn unchanged(path: &Path, file: &fs::File, expected: &fs::Metadata) -> Result<(), String> {
    // The public tool already resolved the path. Recheck both the live name
    // and handle so a displaced file cannot be installed over a new writer.
    let live = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !live.is_file()
        || live.file_type().is_symlink()
        || live.len() != expected.len()
        || opened.len() != expected.len()
        || live.modified().ok() != expected.modified().ok()
        || opened.modified().ok() != expected.modified().ok()
        || live.created().ok() != expected.created().ok()
        || opened.created().ok() != expected.created().ok()
        || live.permissions() != expected.permissions()
        || opened.permissions() != expected.permissions()
    {
        return Err("The file changed while the edit was being prepared; read it and retry".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if live.ino() != expected.ino()
            || live.dev() != expected.dev()
            || live.ctime() != expected.ctime()
            || live.ctime_nsec() != expected.ctime_nsec()
            || opened.ctime() != expected.ctime()
            || opened.ctime_nsec() != expected.ctime_nsec()
        {
            return Err(
                "The file changed while the edit was being prepared; read it and retry".into(),
            );
        }
    }
    Ok(())
}

pub(super) fn edit(
    path: &Path,
    old: &str,
    new: &str,
    mut guard: Option<FileGuard<'_>>,
    cancellation: &Cancellation,
) -> Result<(Value, Option<crate::file_change::Change>), String> {
    check_cancelled(cancellation)?;
    if new.contains('\0') {
        return Err("File content must be UTF-8 text without NUL bytes".into());
    }
    let mut source = fs::File::open(path).map_err(|error| error.to_string())?;
    let metadata = source.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("The path must refer to a regular file".into());
    }
    let (position, ambiguous) = locate(&mut source, old.as_bytes(), cancellation)?;
    unchanged(path, &source, &metadata)?;
    let reason = if position.is_none() {
        Some("old_text was not found; read the file and use exact current text")
    } else if ambiguous {
        Some("old_text occurs more than once; include more exact surrounding text")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Ok((mismatch(&mut source, metadata.len(), reason)?, None));
    }
    if metadata.permissions().readonly() {
        return Err("File is read-only; its content was not changed".into());
    }
    let length = metadata
        .len()
        .checked_sub(old.len() as u64)
        .and_then(|length| length.checked_add(new.len() as u64))
        .ok_or("The replacement exceeds the filesystem length range")?;
    if let Some(guard) = &mut guard {
        guard.capture();
    }
    let result = (|| -> Result<(), String> {
        check_guard(guard.as_ref(), cancellation)?;
        unchanged(path, &source, &metadata)?;
        let temporary = Temporary(crate::disk::temporary_beside(path));
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary.0)
            .map_err(|error| error.to_string())?;
        let staged = (|| -> Result<(), String> {
            source.rewind().map_err(|error| error.to_string())?;
            copy_cancellable(
                &mut source,
                &mut output,
                Some(position.unwrap()),
                cancellation,
            )?;
            for block in new.as_bytes().chunks(BLOCK) {
                check_cancelled(cancellation)?;
                output.write_all(block).map_err(|error| error.to_string())?;
            }
            source
                .seek(SeekFrom::Start(position.unwrap() + old.len() as u64))
                .map_err(|error| error.to_string())?;
            copy_cancellable(&mut source, &mut output, None, cancellation)?;
            output
                .set_permissions(metadata.permissions())
                .map_err(|error| error.to_string())?;
            check_cancelled(cancellation)?;
            output.sync_all().map_err(|error| error.to_string())
        })();
        drop(output);
        staged.map_err(|error| format!("Could not stage file: {error}"))?;
        unchanged(path, &source, &metadata)?;
        drop(source);
        check_guard(guard.as_ref(), cancellation)?;
        fs::rename(&temporary.0, path).map_err(|error| format!("Could not replace file: {error}"))
    })();
    let checkpoint = guard.map(|guard| guard.finish());
    match result {
        Ok(()) => Ok((written(length, checkpoint), None)),
        Err(error) => Ok((failed(error, checkpoint), None)),
    }
}

fn copy_cancellable(
    source: &mut fs::File,
    output: &mut fs::File,
    mut remaining: Option<u64>,
    cancellation: &Cancellation,
) -> Result<(), String> {
    let mut buffer = [0; BLOCK];
    loop {
        check_cancelled(cancellation)?;
        let take = remaining.map_or(BLOCK, |left| left.min(BLOCK as u64) as usize);
        if take == 0 {
            return Ok(());
        }
        let count = source
            .read(&mut buffer[..take])
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| error.to_string())?;
        if let Some(left) = &mut remaining {
            *left -= count as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CancelAfterRead<'a> {
        cancellation: &'a Cancellation,
        read: bool,
    }

    impl Read for CancelAfterRead<'_> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            if self.read {
                return Ok(0);
            }
            self.read = true;
            output[0] = b'x';
            self.cancellation.cancel();
            Ok(1)
        }
    }

    #[test]
    fn streaming_search_checks_cancellation_between_reads() {
        let cancellation = Cancellation::default();
        let source = CancelAfterRead {
            cancellation: &cancellation,
            read: false,
        };
        assert!(
            locate(source, b"x", &cancellation)
                .unwrap_err()
                .contains("cancelled")
        );
    }

    #[test]
    fn matches_across_blocks_and_detects_overlaps() {
        let mut text = vec![b'x'; BLOCK - 2];
        text.extend_from_slice("é日🦀end".as_bytes());
        assert_eq!(
            locate(
                text.as_slice(),
                "é日🦀".as_bytes(),
                &Cancellation::default()
            )
            .unwrap(),
            (Some((BLOCK - 2) as u64), false)
        );
        assert_eq!(
            locate(b"ababa".as_slice(), b"aba", &Cancellation::default()).unwrap(),
            (Some(0), true)
        );
        assert!(locate(b"one\xfftwo".as_slice(), b"one", &Cancellation::default()).is_err());
    }
}
