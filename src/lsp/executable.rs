use super::Profile;
use std::path::{Path, PathBuf};

/// A configured path is taken from the working folder, so Check and every
/// analysis root launch the same file; a bare command name is searched on PATH.
pub(super) fn resolve(profile: &Profile, workspace: &Path) -> Result<PathBuf, String> {
    let configured = Path::new(&profile.executable);
    if configured.is_absolute() || configured.components().count() > 1 {
        return executable(&workspace.join(configured), false)
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
        if let Some(path) = executable(&directory.join(configured), true) {
            return Ok(path);
        }
    }
    Err(format!("Executable not found: {}", profile.executable))
}

/// `profile` with its executable already resolved, for `Client::start`.
pub(super) fn launch(profile: &Profile, workspace: &Path) -> Result<Profile, String> {
    let executable = resolve(profile, workspace)?;
    let mut launch = profile.clone();
    launch.executable = executable.to_string_lossy().into_owned();
    Ok(launch)
}

/// On Windows `.exe` is implied, and a PATH search skips extensionless
/// files: they are shell shims, such as npm's, that Windows cannot start.
fn executable(path: &Path, searched: bool) -> Option<PathBuf> {
    let bare = path.extension().is_none();
    let given = (!(cfg!(windows) && searched && bare)).then(|| path.to_path_buf());
    let implied = (cfg!(windows) && bare).then(|| path.with_extension("exe"));
    given
        .into_iter()
        .chain(implied)
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn path_search_skips_windows_shims_but_honors_explicit_paths() {
        let directory = Directory::new();
        let shim = directory.path().join("server");
        std::fs::write(&shim, "#!/bin/sh\n").unwrap();
        let mut profile = Profile::draft_guided_rust();
        profile.executable = "server".into();
        profile.environment.insert(
            "PATH".into(),
            directory.path().to_string_lossy().into_owned(),
        );
        if cfg!(windows) {
            let missing = resolve(&profile, directory.path()).unwrap_err();
            assert_eq!(missing, "Executable not found: server");
            let program = directory.path().join("server.exe");
            std::fs::write(&program, "").unwrap();
            assert_eq!(resolve(&profile, directory.path()).unwrap(), program);
        } else {
            assert_eq!(resolve(&profile, directory.path()).unwrap(), shim);
        }
        profile.executable = shim.to_string_lossy().into_owned();
        assert_eq!(resolve(&profile, directory.path()).unwrap(), shim);
    }

    #[test]
    fn relative_path_resolves_from_the_working_folder() {
        let directory = Directory::new();
        let name = format!("server{}", std::env::consts::EXE_SUFFIX);
        std::fs::create_dir_all(directory.path().join("tools")).unwrap();
        std::fs::create_dir_all(directory.path().join("nested")).unwrap();
        std::fs::write(directory.path().join("tools").join(&name), "").unwrap();
        let mut profile = Profile::draft_guided_rust();
        profile.executable = format!("tools/{name}");
        let launch = launch(&profile, directory.path()).unwrap();
        assert_eq!(
            PathBuf::from(&launch.executable),
            directory.path().join("tools").join(&name)
        );
        assert!(resolve(&profile, &directory.path().join("nested")).is_err());
        assert_eq!(
            resolve(&launch, &directory.path().join("nested")).unwrap(),
            PathBuf::from(&launch.executable)
        );
    }
}
