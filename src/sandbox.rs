//! A per-launch filesystem boundary for user shell commands on Windows.
//! Native process launch consumes the trusted environment marker and removes
//! it before the target process starts.

#[cfg(any(windows, test))]
use crate::json::Value;
use crate::permissions::{Access, Kind, Level};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const MARKER: &str = "JECODE_INTERNAL_SANDBOX_POLICY";

#[derive(Debug)]
pub struct Policy {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
    cargo_home: Option<PathBuf>,
    git_config: Option<PathBuf>,
}

impl Policy {
    /// The embedded interpreter is an exact read-only executable, not a grant
    /// to configuration, credentials or the rest of its parent directory.
    pub fn allow_owned_shell(&mut self) -> Result<(), String> {
        let executable = crate::shell::executable()?;
        let executable = std::fs::canonicalize(executable)
            .map_err(|error| format!("Could not resolve the owned shell: {error}"))?;
        if !self.read.contains(&executable) {
            self.read.push(executable);
        }
        Ok(())
    }

    /// Full has no confinement. Read cannot launch a user command.
    pub fn for_command(
        level: Level,
        root: &Path,
        temporary: Option<&Path>,
        access: Option<&Access>,
    ) -> Result<Option<Self>, String> {
        match level {
            Level::Full => return Ok(None),
            Level::Read => {
                return Err("Commands are unavailable at the Read permission level".into());
            }
            Level::Workspace => {}
        }
        let root = canonical_directory(root)?;
        let mut policy = Self {
            read: vec![root.clone()],
            write: vec![root.clone()],
            cargo_home: None,
            git_config: None,
        };
        if let Some(temporary) = temporary {
            let temporary = canonical_directory(temporary)?;
            policy.cargo_home = Some(temporary.join("cargo"));
            policy.git_config = Some(temporary.join("gitconfig"));
            policy.read.push(temporary.clone());
            policy.write.push(temporary);
        }
        if let Some(access) = access
            && access.kind == Kind::Runs
            && access.outside
        {
            let place = access
                .place
                .as_deref()
                .ok_or("Approved command has no working directory")?;
            let place = canonical_directory(Path::new(place))?;
            policy.read.push(place.clone());
            policy.write.push(place);
        }
        Ok(Some(policy))
    }

    pub fn attach(&self, command: &mut Command) -> Result<(), String> {
        #[cfg(not(windows))]
        {
            let _ = command;
            Err("Workspace command confinement is supported only on Windows. Choose /permissions full to run without confinement on this platform.".into())
        }
        #[cfg(windows)]
        {
            command.env(MARKER, self.encode());
            Ok(())
        }
    }

    /// A recursive workspace write grant cannot safely contain credentials
    /// or other saved conversations in Jecode's configuration directory.
    pub fn protect_session_home(&self, home: Option<&Path>) -> Result<(), String> {
        let Some(home) = home else { return Ok(()) };
        let home = canonical_directory(home)?;
        if self.write.iter().any(|root| home.starts_with(root)) {
            return Err("Jecode's configuration or other sessions are inside the working folder. Workspace commands cannot be confined here; choose a narrower working folder or /permissions full.".into());
        }
        Ok(())
    }

    /// Existing Rustup data and Cargo launchers are read-only runtime inputs.
    /// Never grant the Cargo home itself: it can contain credentials.
    pub fn allow_installed_rust_tools(
        &mut self,
        protected_home: Option<&Path>,
    ) -> Result<(), String> {
        let profile = std::env::var_os("USERPROFILE").map(PathBuf::from);
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
        Ok(())
    }

    fn allow_existing_read_directory(
        &mut self,
        path: &Path,
        protected_home: Option<&Path>,
    ) -> Result<(), String> {
        if !path.is_dir() {
            return Ok(());
        }
        let path = canonical_directory(path)?;
        if let Some(home) = protected_home {
            let home = canonical_directory(home)?;
            if path.starts_with(&home) || home.starts_with(&path) {
                return Err("A runtime read root overlaps Jecode's configuration directory; Workspace command launch was refused".into());
            }
        }
        if !self.read.iter().any(|root| path.starts_with(root)) {
            self.read.push(path);
        }
        Ok(())
    }

    #[cfg(any(windows, test))]
    fn encode(&self) -> String {
        Value::object([
            ("read", paths(&self.read)),
            ("write", paths(&self.write)),
            ("deny", Value::Array(Vec::new())),
            ("network", Value::Bool(true)),
        ])
        .encode()
    }
}

pub fn apply(command: &mut Command, policy: Option<&Policy>) -> Result<(), String> {
    if let Some(policy) = policy {
        confined_environment(command, policy, std::env::vars_os());
        policy.attach(command)?;
    } else {
        // Full has its inherited environment but never accepts a forged marker.
        command.env_remove(MARKER);
    }
    Ok(())
}

fn confined_environment(
    command: &mut Command,
    policy: &Policy,
    parent: impl IntoIterator<Item = (OsString, OsString)>,
) {
    let explicit = command
        .get_envs()
        .filter(|(key, _)| !key.to_string_lossy().eq_ignore_ascii_case(MARKER))
        .map(|(key, value)| (key.to_os_string(), value.map(OsStr::to_os_string)))
        .collect::<Vec<_>>();
    command.env_clear();
    for (key, value) in parent {
        if allowed_parent_variable(&key) {
            command.env(key, value);
        }
    }
    for (key, value) in &explicit {
        match value {
            Some(value) => {
                command.env(key, value);
            }
            None => {
                command.env_remove(key);
            }
        }
    }
    if let Some(cargo_home) = &policy.cargo_home
        && !explicit
            .iter()
            .any(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case("CARGO_HOME"))
    {
        command.env("CARGO_HOME", crate::permissions::plain(cargo_home));
    }
    if let Some(git_config) = &policy.git_config
        && !explicit.iter().any(|(key, _)| {
            key.to_string_lossy()
                .eq_ignore_ascii_case("GIT_CONFIG_GLOBAL")
        })
    {
        command.env("GIT_CONFIG_GLOBAL", crate::permissions::plain(git_config));
    }
}

fn allowed_parent_variable(key: &OsStr) -> bool {
    let key = key.to_string_lossy();
    [
        "SystemRoot",
        "WINDIR",
        "SystemDrive",
        "ComSpec",
        "PATH",
        "PATHEXT",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "USERNAME",
        "USERDOMAIN",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "TMPDIR",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "CommonProgramFiles",
        "PROCESSOR_ARCHITECTURE",
        "NUMBER_OF_PROCESSORS",
        "OS",
        "RUSTUP_HOME",
        "RUSTUP_TOOLCHAIN",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_TARGET",
        "RUSTC",
        "RUSTDOC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CC",
        "CXX",
        "AR",
        "LINKER",
    ]
    .iter()
    .any(|allowed| key.eq_ignore_ascii_case(allowed))
}

#[cfg(any(windows, test))]
fn paths(paths: &[PathBuf]) -> Value {
    Value::Array(
        paths
            .iter()
            .map(|path| Value::string(path.to_string_lossy()))
            .collect(),
    )
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(path).map_err(|error| {
        format!(
            "Could not resolve sandbox directory {}: {error}",
            path.display()
        )
    })?;
    if !path.is_dir() {
        return Err(format!(
            "Sandbox path is not a directory: {}",
            path.display()
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
