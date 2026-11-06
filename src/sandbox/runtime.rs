//! Read-only runtime inputs for Workspace commands.
use super::{Policy, canonical_directory, user_home};
use std::path::{Path, PathBuf};

impl Policy {
    /// Existing Rustup data and Cargo launchers are read-only runtime inputs.
    /// Never grant the Cargo home itself: it can contain credentials.
    pub fn allow_installed_rust_tools(
        &mut self,
        protected_home: Option<&Path>,
    ) -> Result<(), String> {
        let profile = user_home();
        let rustup = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| profile.as_ref().map(|profile| profile.join(".rustup")));
        let cargo = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| profile.map(|profile| profile.join(".cargo")));
        for path in [rustup, cargo.map(|cargo| cargo.join("bin"))]
            .into_iter()
            .flatten()
        {
            self.allow_existing_read_directory(&path, protected_home)?;
        }
        self.allow_cargo_configuration(protected_home)?;
        Ok(())
    }

    /// Cargo reads the configuration in every ancestor of the project and stops
    /// when it cannot open one. These exact files hold settings; publishing
    /// tokens live in `credentials.toml`, which stays unreadable.
    pub(super) fn allow_cargo_configuration(
        &mut self,
        protected_home: Option<&Path>,
    ) -> Result<(), String> {
        let Some(root) = self.read.first().cloned() else {
            return Ok(());
        };
        let protected = protected_home.map(canonical_directory).transpose()?;
        for ancestor in root.ancestors().skip(1) {
            for name in ["config.toml", "config"] {
                let spelled = ancestor.join(".cargo").join(name);
                if let Ok(file) = std::fs::canonicalize(&spelled)
                    && file.is_file()
                    && !self.read.contains(&file)
                {
                    if protected
                        .as_ref()
                        .is_some_and(|home| file.starts_with(home))
                    {
                        return Err("A Cargo configuration resolves inside Jecode's configuration directory; Workspace command launch was refused".into());
                    }
                    // An ancestor can contain an untrusted link, at `.cargo` or at the
                    // file, to something private. The ancestor itself is canonical, so
                    // a different resolved path means a link: it grants nothing.
                    if file == spelled {
                        self.read.push(file);
                    }
                }
            }
        }
        Ok(())
    }

    /// On Unix, the folders on PATH inside the user's home hold tools such as
    /// `~/.local/bin`. They are read-only runtime inputs; system locations are
    /// already readable, and the home itself is never granted.
    pub fn allow_path_tools(&mut self, protected_home: Option<&Path>) -> Result<(), String> {
        if cfg!(windows) {
            return Ok(());
        }
        let (Some(home), Some(search)) = (user_home(), std::env::var_os("PATH")) else {
            return Ok(());
        };
        // A service account's home may not exist; nothing on PATH is then inside it.
        let Ok(home) = canonical_directory(&home) else {
            return Ok(());
        };
        let protected = protected_home.map(canonical_directory).transpose()?;
        for directory in std::env::split_paths(&search) {
            let Ok(directory) = canonical_directory(&directory) else {
                continue;
            };
            let overlaps = protected.as_ref().is_some_and(|protected| {
                directory.starts_with(protected) || protected.starts_with(&directory)
            });
            if directory != home && directory.starts_with(&home) && !overlaps {
                self.allow_existing_read_directory(&directory, protected_home)?;
            }
        }
        Ok(())
    }
}
