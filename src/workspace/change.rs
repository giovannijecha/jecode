//! Prepared content is immutable. Preparing a proposal never creates a file.
use super::{Budget, Error, MAX_FILE_BYTES, Workspace, diff, platform, relative};
use std::{
    fs::File,
    io::{self, Read, Seek},
    time::SystemTime,
};

pub const MAX_CHANGE_BYTES: usize = 32 * 1024;
#[derive(Clone, Debug)]
pub struct Preview {
    pub path: String,
    pub create: bool,
    /// Complete bounded change, with unchanged context on either side.
    pub diff: String,
    pub added: usize,
    pub removed: usize,
}
#[derive(Debug)]
pub struct ChangeError(pub String);
impl std::fmt::Display for ChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ChangeError {}
impl From<Error> for ChangeError {
    fn from(value: Error) -> Self {
        Self(value.to_string())
    }
}
impl From<io::Error> for ChangeError {
    fn from(error: io::Error) -> Self {
        Self(
            if error.kind() == io::ErrorKind::Unsupported
                && error.to_string() == "filesystem does not support no-replace renames"
            {
                "filesystem does not support no-replace renames; proposed content not published"
                    .into()
            } else {
                "file unavailable or unsupported for safe text changes; proposed content not published".into()
            },
        )
    }
}
pub(super) type Identity = (u64, u64);
pub(super) struct Snapshot {
    pub id: Identity,
    pub modified: SystemTime,
    pub text: String,
}
pub struct Change {
    pub(super) parent: Identity,
    pub(super) before: Option<Snapshot>,
    pub(super) after: String,
    pub(super) preview: Preview,
}
impl Change {
    pub fn preview(&self) -> &Preview {
        &self.preview
    }
}
impl Workspace {
    pub fn prepare_create(
        &self,
        path: &str,
        content: &str,
        budget: &Budget<'_>,
    ) -> Result<Change, ChangeError> {
        validate_text(content)?;
        if content.len() > MAX_CHANGE_BYTES {
            return fail("new content exceeds the 32 KiB proposal limit");
        }
        let (path, parent, name) = self.change_parent(path, budget)?;
        match platform::edit_open(&parent.file, &name) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            _ => {
                return fail(
                    "create_file requires an absent file; existing entries are never overwritten",
                );
            }
        }
        let preview = diff::preview(&path, None, content)?;
        budget.check()?;
        Ok(Change {
            parent: platform::identity(&parent.file)?,
            before: None,
            after: content.into(),
            preview,
        })
    }
    pub fn prepare_edit(
        &self,
        path: &str,
        old: &str,
        new: &str,
        budget: &Budget<'_>,
    ) -> Result<Change, ChangeError> {
        if old.len() + new.len() > MAX_CHANGE_BYTES {
            return fail("old_text plus new_text must fit 32 KiB");
        }
        validate_text(new)?;
        let (path, parent, name) = self.change_parent(path, budget)?;
        let mut file = platform::edit_open(&parent.file, &name)?;
        platform::editable(&file)?;
        let before = snapshot(&mut file, budget)?;
        if old.is_empty() && !before.text.is_empty() {
            return fail("empty old_text is only valid for an empty file");
        }
        let Some(at) = before.text.find(old) else {
            return fail("old_text does not match; read the file and propose an exact replacement");
        };
        if !old.is_empty()
            && before.text[at + old.chars().next().unwrap().len_utf8()..].contains(old)
        {
            return fail(
                "old_text is ambiguous; include enough context to identify one occurrence",
            );
        }
        let mut after = before.text.clone();
        after.replace_range(at..at + old.len(), new);
        if after == before.text {
            return fail("replacement makes no change");
        }
        if after.len() > MAX_FILE_BYTES {
            return fail("result exceeds the 1 MiB text file limit");
        }
        validate_text(&after)?;
        let preview = diff::preview(&path, Some(&before.text), &after)?;
        budget.check()?;
        Ok(Change {
            parent: platform::identity(&parent.file)?,
            before: Some(before),
            after,
            preview,
        })
    }
    pub(super) fn change_parent(
        &self,
        path: &str,
        budget: &Budget<'_>,
    ) -> Result<(String, super::Opened, String), ChangeError> {
        budget.check()?;
        let path = relative(path)?;
        if path == "." || path.chars().any(bidi) {
            return fail("change requires an unambiguous relative file path");
        }
        let (parent, name) = path.rsplit_once('/').unwrap_or((".", &path));
        let opened = platform::open(&self.root, parent, true)?;
        let name = name.to_owned();
        Ok((path, opened, name))
    }
}
pub(super) fn snapshot(file: &mut File, budget: &Budget<'_>) -> Result<Snapshot, ChangeError> {
    budget.check()?;
    let before = file.metadata()?;
    if !before.is_file() || before.len() > MAX_FILE_BYTES as u64 {
        return Err(Error::Size.into());
    }
    let modified = before.modified()?;
    file.rewind()?;
    let mut bytes = Vec::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        budget.check()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_FILE_BYTES {
            return Err(Error::Size.into());
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    let after = file.metadata()?;
    if before.len() != after.len() || modified != after.modified()? {
        return fail("file changed during validation; prepare a new proposal");
    }
    let text = String::from_utf8(bytes).map_err(|_| ChangeError(Error::Text.to_string()))?;
    validate_text(&text)?;
    Ok(Snapshot {
        id: platform::identity(file)?,
        modified,
        text,
    })
}
pub(super) fn matches(
    file: &mut File,
    expected: &Snapshot,
    budget: &Budget<'_>,
) -> Result<(), ChangeError> {
    let current = snapshot(file, budget)?;
    if current.id != expected.id
        || current.modified != expected.modified
        || current.text != expected.text
    {
        return fail("file changed since the preview; prepare a new proposal");
    }
    Ok(())
}
fn validate_text(text: &str) -> Result<(), ChangeError> {
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t') || bidi(c))
        || text.replace("\r\n", "").contains('\r')
    {
        return fail("changes require UTF-8 text with LF/CRLF, without binary or bidi controls");
    }
    Ok(())
}
fn bidi(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}
pub(super) fn fail<T>(text: &str) -> Result<T, ChangeError> {
    Err(ChangeError(text.into()))
}
