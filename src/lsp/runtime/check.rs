use crate::cancel::Cancellation;
use crate::lsp::client::Client;
use crate::lsp::{CheckObservation, Profile};
use std::path::Path;

pub fn check(
    profile: &Profile,
    workspace: &Path,
    cancel: &Cancellation,
) -> Result<CheckObservation, String> {
    profile.validate()?;
    let root = profile.root_override.as_ref().map_or_else(
        || workspace.to_path_buf(),
        |root| {
            if root.is_absolute() {
                root.clone()
            } else {
                workspace.join(root)
            }
        },
    );
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    if !root.is_dir() {
        return Err("LSP check root must be a directory".into());
    }
    let executable = crate::lsp::executable::resolve(profile, &root)?;
    let mut client = Client::start(profile, &root, cancel, &Cancellation::default(), 1)?;
    let capabilities = client.capabilities.clone();
    client.shutdown();
    Ok(CheckObservation {
        capabilities,
        root,
        executable: executable.to_string_lossy().into_owned(),
        fingerprint: profile.analysis_fingerprint(),
    })
}
