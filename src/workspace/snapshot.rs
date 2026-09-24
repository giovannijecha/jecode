//! Owned snapshots and streaming validation for larger files.
use super::change::{ChangeError, Identity, bidi, fail};
use super::{Budget, Error, MAX_FILE_BYTES, platform};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

pub(super) struct OwnedFile {
    pub file: File,
    parent: File,
    name: String,
    pub path: PathBuf,
}
impl OwnedFile {
    pub fn create(parent: &File, directory: &Path, kind: &str) -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let owned_parent = parent.try_clone()?;
        for _ in 0..100 {
            let name = format!(
                ".jecode-{kind}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            match platform::create(parent, &name) {
                Ok(file) => {
                    return Ok(Self {
                        file,
                        parent: owned_parent,
                        path: directory.join(&name),
                        name,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::ErrorKind::AlreadyExists.into())
    }
}
impl Drop for OwnedFile {
    fn drop(&mut self) {
        let _ = platform::remove_owned(&self.parent, &self.file, &self.name);
    }
}
pub(super) enum Original {
    Memory(String),
    Staged(OwnedFile),
}
pub(super) struct Snapshot {
    pub id: Identity,
    pub modified: SystemTime,
    pub len: u64,
    pub original: Original,
}
pub(super) fn snapshot(
    file: &mut File,
    parent: &File,
    directory: &Path,
    stage_result: bool,
    budget: &Budget<'_>,
) -> Result<Snapshot, ChangeError> {
    budget.check()?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(Error::Unavailable.into());
    }
    let modified = before.modified()?;
    file.rewind()?;
    let staged = stage_result || before.len() > MAX_FILE_BYTES as u64;
    let mut original = if staged {
        let staged = OwnedFile::create(parent, directory, "snapshot")?;
        platform::metadata_to(file, &staged.file)?;
        Original::Staged(staged)
    } else {
        Original::Memory(String::new())
    };
    let mut memory = Vec::new();
    let mut validator = TextValidator::default();
    let mut buffer = [0; 16 * 1024];
    let mut length = 0u64;
    loop {
        budget.check()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        length = length.checked_add(n as u64).ok_or(Error::Size)?;
        validator.push(&buffer[..n])?;
        match &mut original {
            Original::Memory(_) => memory.extend_from_slice(&buffer[..n]),
            Original::Staged(staged) => staged.file.write_all(&buffer[..n])?,
        }
    }
    validator.finish()?;
    if let Original::Memory(text) = &mut original {
        *text = String::from_utf8(memory).map_err(|_| Error::Text)?;
    }
    let after = file.metadata()?;
    if before.len() != length || before.len() != after.len() || modified != after.modified()? {
        return fail("file changed during validation; prepare a new proposal");
    }
    Ok(Snapshot {
        id: platform::identity(file)?,
        modified,
        len: length,
        original,
    })
}
pub(super) fn matches(
    file: &mut File,
    expected: &mut Snapshot,
    budget: &Budget<'_>,
) -> Result<(), ChangeError> {
    budget.check()?;
    let before = file.metadata()?;
    if platform::identity(file)? != expected.id
        || before.len() != expected.len
        || before.modified()? != expected.modified
    {
        return fail("file changed since the preview; prepare a new proposal");
    }
    file.rewind()?;
    if let Original::Staged(staged) = &mut expected.original {
        staged.file.rewind()?;
    }
    let mut buffer = [0; 16 * 1024];
    let mut position = 0;
    loop {
        budget.check()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        let equal = match &mut expected.original {
            Original::Memory(text) => {
                text.as_bytes().get(position..position + n) == Some(&buffer[..n])
            }
            Original::Staged(staged) => {
                let mut other = [0; 16 * 1024];
                staged.file.read_exact(&mut other[..n])?;
                other[..n] == buffer[..n]
            }
        };
        if !equal {
            return fail("file changed since the preview; prepare a new proposal");
        }
        position += n;
    }
    let after = file.metadata()?;
    if position as u64 != expected.len
        || after.len() != before.len()
        || after.modified()? != expected.modified
    {
        return fail("file changed since the preview; prepare a new proposal");
    }
    Ok(())
}
pub(super) fn copy_replacement(
    snapshot: &mut Snapshot,
    target: &mut File,
    at: u64,
    old_len: u64,
    new: &str,
    budget: &Budget<'_>,
) -> Result<(), ChangeError> {
    let Original::Staged(staged) = &mut snapshot.original else {
        return fail("missing staged original");
    };
    staged.file.rewind()?;
    copy_exact(&mut staged.file, target, at, budget)?;
    for chunk in new.as_bytes().chunks(16 * 1024) {
        budget.check()?;
        target.write_all(chunk)?;
    }
    staged.file.seek(SeekFrom::Start(at + old_len))?;
    copy_exact(
        &mut staged.file,
        target,
        snapshot.len - at - old_len,
        budget,
    )
}
fn copy_exact(
    source: &mut File,
    target: &mut File,
    mut remaining: u64,
    budget: &Budget<'_>,
) -> Result<(), ChangeError> {
    let mut buffer = [0; 16 * 1024];
    while remaining != 0 {
        budget.check()?;
        let size = remaining.min(buffer.len() as u64) as usize;
        let n = source.read(&mut buffer[..size])?;
        if n == 0 {
            return fail("staged original changed before publication");
        }
        target.write_all(&buffer[..n])?;
        remaining -= n as u64;
    }
    Ok(())
}
pub(super) fn scan_match(
    file: &mut File,
    old: &str,
    budget: &Budget<'_>,
) -> Result<u64, ChangeError> {
    if old.is_empty() {
        return fail("empty old_text is only valid for an empty file");
    }
    let pattern = old.as_bytes();
    let mut failure = vec![0; pattern.len()];
    for i in 1..pattern.len() {
        let mut j = failure[i - 1];
        while j > 0 && pattern[i] != pattern[j] {
            j = failure[j - 1];
        }
        if pattern[i] == pattern[j] {
            j += 1;
        }
        failure[i] = j;
    }
    file.rewind()?;
    let mut matched = 0;
    let mut offset = 0u64;
    let mut found = None;
    let mut buffer = [0; 16 * 1024];
    loop {
        budget.check()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        for &byte in &buffer[..n] {
            while matched > 0 && byte != pattern[matched] {
                matched = failure[matched - 1];
            }
            if byte == pattern[matched] {
                matched += 1;
            }
            if matched == pattern.len() {
                if found.is_some() {
                    return fail(
                        "old_text is ambiguous; include enough context to identify one occurrence",
                    );
                }
                found = Some(offset + 1 - pattern.len() as u64);
                matched = failure[matched - 1];
            }
            offset += 1;
        }
    }
    found.ok_or_else(|| {
        ChangeError(
            "old_text does not match; read the file and propose an exact replacement".into(),
        )
    })
}
#[derive(Default)]
struct TextValidator {
    pending: Vec<u8>,
    previous_cr: bool,
}
impl TextValidator {
    fn push(&mut self, bytes: &[u8]) -> Result<(), ChangeError> {
        let mut chunk = std::mem::take(&mut self.pending);
        chunk.extend_from_slice(bytes);
        let valid_len = match std::str::from_utf8(&chunk) {
            Ok(_) => chunk.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => return Err(Error::Text.into()),
        };
        let text = std::str::from_utf8(&chunk[..valid_len]).map_err(|_| Error::Text)?;
        for c in text.chars() {
            if self.previous_cr && c != '\n' {
                return Err(Error::Text.into());
            }
            self.previous_cr = c == '\r';
            if (c.is_control() && !matches!(c, '\n' | '\r' | '\t')) || bidi(c) {
                return Err(Error::Text.into());
            }
        }
        self.pending.extend_from_slice(&chunk[valid_len..]);
        Ok(())
    }
    fn finish(&self) -> Result<(), ChangeError> {
        if self.pending.is_empty() && !self.previous_cr {
            Ok(())
        } else {
            Err(Error::Text.into())
        }
    }
}
