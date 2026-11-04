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
    /// Real toolchain homes passed to the confined process by name.
    runtime_homes: Vec<(&'static str, PathBuf)>,
    /// Installed Node.js runtime locations. Empty when Node.js is absent.
    node_paths: Vec<PathBuf>,
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
            runtime_homes: Vec::new(),
            node_paths: Vec::new(),
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

    /// Node version managers keep runtimes in the user profile, but a confined
    /// process sees its own container as LOCALAPPDATA. Pass each manager's real
    /// home and grant it read-only, so shims resolve the selected runtime. Any
    /// installed runtime, including the first `node.exe` on PATH, also needs
    /// its drive roots enumerable; see `enumerable_roots`.
    pub fn allow_installed_node_tools(
        &mut self,
        protected_home: Option<&Path>,
    ) -> Result<(), String> {
        self.allow_node_tools_from(|name| std::env::var_os(name), protected_home)
    }

    fn allow_node_tools_from(
        &mut self,
        variable: impl Fn(&str) -> Option<OsString>,
        protected_home: Option<&Path>,
    ) -> Result<(), String> {
        let local = variable("LOCALAPPDATA").map(PathBuf::from);
        let homes = [
            (
                "VOLTA_HOME",
                variable("VOLTA_HOME")
                    .map(PathBuf::from)
                    .or_else(|| local.map(|local| local.join("Volta"))),
            ),
            ("NVM_HOME", variable("NVM_HOME").map(PathBuf::from)),
            ("NVM_SYMLINK", variable("NVM_SYMLINK").map(PathBuf::from)),
            ("FNM_DIR", variable("FNM_DIR").map(PathBuf::from)),
        ];
        for (name, path) in homes {
            let Some(path) = path.filter(|path| path.is_absolute() && path.is_dir()) else {
                continue;
            };
            self.allow_existing_read_directory(&path, protected_home)?;
            self.add_node_path(&path);
            self.runtime_homes.push((name, path));
        }
        if let Some(directory) = variable("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .find(|directory| directory.is_absolute() && directory.join("node.exe").is_file())
        }) {
            self.add_node_path(&directory);
        }
        Ok(())
    }

    /// Both spellings matter: a linked install resolves through the link's
    /// drive and then through its target's drive.
    fn add_node_path(&mut self, path: &Path) {
        self.node_paths.push(path.to_path_buf());
        if let Ok(target) = std::fs::canonicalize(path) {
            self.node_paths.push(target);
        }
    }

    /// Node.js resolves module and script paths by checking every component
    /// from the drive root, and the root is the one component confinement
    /// leaves unreadable. The Windows policy can grant listing only for a
    /// whole tree, so when Node.js is installed the drives holding writable
    /// folders and the runtime become listable: names and metadata, never
    /// file contents or writes.
    #[cfg(any(windows, test))]
    fn enumerable_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        if self.node_paths.is_empty() {
            return roots;
        }
        for path in self.write.iter().chain(&self.node_paths) {
            let Some(root) = path.ancestors().last().filter(|root| root.has_root()) else {
                continue;
            };
            let root = PathBuf::from(crate::permissions::plain(root));
            if !roots
                .iter()
                .any(|known| known.as_os_str().eq_ignore_ascii_case(root.as_os_str()))
            {
                roots.push(root);
            }
        }
        roots
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
            ("enumerate", paths(&self.enumerable_roots())),
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
    for (name, home) in &policy.runtime_homes {
        if !explicit
            .iter()
            .any(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case(name))
        {
            command.env(name, home);
        }
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
#[cfg(all(test, windows))]
mod windows_tests;
