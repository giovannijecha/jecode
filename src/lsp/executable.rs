use super::Profile;
use std::path::{Path, PathBuf};

pub(super) fn resolve(profile: &Profile, root: &Path) -> Result<PathBuf, String> {
    let configured = Path::new(&profile.executable);
    if configured.is_absolute() || configured.components().count() > 1 {
        let path = if configured.is_absolute() {
            configured.to_path_buf()
        } else {
            root.join(configured)
        };
        return executable(&path)
            .ok_or_else(|| format!("Executable not found: {}", profile.executable));
    }
    let path_value = profile
        .environment
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value.as_str());
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let search = path_value
        .map(std::ffi::OsString::from)
        .unwrap_or(inherited);
    for directory in std::env::split_paths(&search) {
        if let Some(path) = executable(&directory.join(configured)) {
            return Ok(path);
        }
    }
    Err(format!("Executable not found: {}", profile.executable))
}

fn executable(path: &Path) -> Option<PathBuf> {
    let candidates = vec![path.to_path_buf()];
    #[cfg(windows)]
    let candidates = {
        let mut candidates = candidates;
        if path.extension().is_none() {
            candidates.push(path.with_extension("exe"));
        }
        candidates
    };
    candidates.into_iter().find(|candidate| candidate.is_file())
}
