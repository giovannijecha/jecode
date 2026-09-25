//! Prepared content is immutable; large originals use owned, disposable snapshots.
use super::{
    Budget, Error, MAX_FILE_BYTES, Workspace, diff, platform,
    snapshot::{Original, Snapshot, scan_match, snapshot, validate_replacement},
};
use std::io;

#[derive(Clone, Debug)]
pub struct Preview {
    pub path: String,
    pub create: bool,
    /// Bounded leading lines of the change. Omission counts describe the rest.
    pub diff: String,
    pub added: usize,
    pub removed: usize,
    pub omitted_lines: usize,
    pub omitted_bytes: usize,
}
#[derive(Debug)]
pub struct ChangeError(pub String, pub Option<String>);
impl std::fmt::Display for ChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ChangeError {}
impl From<Error> for ChangeError {
    fn from(value: Error) -> Self {
        Self(value.to_string(), None)
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
            None,
        )
    }
}
pub(super) type Identity = (u64, u64);
pub(super) enum After {
    Complete(String),
    Replacement { at: u64, old_len: u64, new: String },
}
pub struct Change {
    pub(super) parent: Identity,
    pub(super) before: Option<Snapshot>,
    pub(super) after: After,
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
        let (path, parent, name) = self.change_parent(path, budget)?;
        match platform::edit_open(&parent.file, &name) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            _ => {
                return fail(
                    "create_file requires an absent file; existing entries are never overwritten",
                );
            }
        }
        let preview = diff::preview(&path, None, content);
        budget.check()?;
        Ok(Change {
            parent: platform::identity(&parent.file)?,
            before: None,
            after: After::Complete(content.into()),
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
        validate_text(new)?;
        let (path, parent, name) = self.change_parent(path, budget)?;
        let mut file = platform::edit_open(&parent.file, &name)?;
        platform::editable(&file)?;
        let stage_result =
            file.metadata()?.len().saturating_add(new.len() as u64) > MAX_FILE_BYTES as u64;
        let mut before = snapshot(&mut file, &parent.file, stage_result, budget)?;
        if old.is_empty() && before.len != 0 {
            return fail("empty old_text is only valid for an empty file");
        }
        if old == new {
            return fail("replacement makes no change");
        }
        let (after, preview) = match &mut before.original {
            Original::Memory(text) => {
                let at = unique_match(text, old)?;
                let mut after = text.clone();
                after.replace_range(at..at + old.len(), new);
                validate_text(&after)?;
                let preview = diff::preview(&path, Some(text), &after);
                (After::Complete(after), preview)
            }
            Original::Staged(staged) => {
                let at = scan_match(&mut staged.file, old, budget)?;
                validate_replacement(
                    &mut staged.file,
                    before.len,
                    at,
                    old.len() as u64,
                    new,
                    budget,
                )?;
                let preview = diff::replacement_preview(&path, at, old, new);
                (
                    After::Replacement {
                        at,
                        old_len: old.len() as u64,
                        new: new.into(),
                    },
                    preview,
                )
            }
        };
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
        let path = self.resolve(path)?.display;
        if path == "." || path.chars().any(bidi) {
            return fail("change requires an unambiguous file path");
        }
        let (parent, name) = path.rsplit_once('/').unwrap_or((".", &path));
        if name.is_empty() {
            return fail("change requires a file name, not a filesystem root");
        }
        let parent = if parent.is_empty() { "/" } else { parent };
        let parent = if parent.ends_with(':') {
            format!("{parent}/")
        } else {
            parent.into()
        };
        let opened = self.open_location(&self.resolve(&parent)?, true)?;
        let name = name.to_owned();
        Ok((path, opened, name))
    }
}
fn unique_match(text: &str, old: &str) -> Result<usize, ChangeError> {
    let Some(at) = text.find(old) else {
        return fail("old_text does not match; read the file and propose an exact replacement");
    };
    if !old.is_empty() && text[at + old.chars().next().unwrap().len_utf8()..].contains(old) {
        return fail("old_text is ambiguous; include enough context to identify one occurrence");
    }
    Ok(at)
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
pub(super) fn bidi(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}
pub(super) fn fail<T>(text: &str) -> Result<T, ChangeError> {
    Err(ChangeError(text.into(), None))
}

#[cfg(test)]
#[path = "change_tests.rs"]
mod tests;
