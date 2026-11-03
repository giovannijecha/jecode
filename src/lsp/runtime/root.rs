use crate::lsp::Profile;
use std::path::{Path, PathBuf};

pub(super) fn choose_root(
    workspace: &Path,
    path: &Path,
    profile: &Profile,
) -> Result<(PathBuf, String), String> {
    if let Some(root) = &profile.root_override {
        let root = if root.is_absolute() {
            root.clone()
        } else {
            workspace.join(root)
        };
        let root = std::fs::canonicalize(root)
            .map_err(|e| format!("LSP root override unavailable: {e}"))?;
        if !root.is_dir() {
            return Err("LSP root override must be a directory".into());
        }
        return Ok((root, "Explicit root override".into()));
    }
    let workspace = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let mut current = path.parent();
    while let Some(folder) = current {
        if !folder.starts_with(&workspace) {
            break;
        }
        if let Some(marker) = profile
            .root_markers
            .iter()
            .find(|marker| folder.join(marker).exists())
        {
            return Ok((folder.to_path_buf(), format!("Found root marker {marker}")));
        }
        current = folder.parent();
    }
    Ok((workspace, "Selected workspace root".into()))
}
