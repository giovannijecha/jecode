use super::Area;
use crate::disk::owned;

/// A fully inspected, owned session area ready for one deletion attempt.
pub(crate) struct Removal(owned::Removal);

impl Removal {
    pub(crate) fn remove(self) -> Result<(usize, usize), String> {
        let removed = self.0.remove()?;
        Ok((removed.files, removed.directories))
    }
}

impl Area {
    /// Inspect an existing area without creating a missing one. The marker and
    /// root are included in the removal count and ordered after their contents.
    pub(crate) fn prepare_removal(&self) -> Result<Option<Removal>, String> {
        Ok(self
            .owned_directory(self.path.clone())
            .prepare_removal()?
            .map(Removal))
    }
}
