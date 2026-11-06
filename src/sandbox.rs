//! A per-launch filesystem boundary for user shell commands: a Process Security
//! Environment on Windows, Landlock on Linux and Seatbelt on macOS. Native
//! process launch consumes the trusted environment marker and removes it before
//! the target process starts.

#[cfg(any(windows, test))]
use crate::json::Value;
use crate::permissions::{Access, Kind, Level};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(any(target_os = "macos", test))]
mod apple;
mod runtime;
#[cfg(any(unix, test))]
mod unix;

pub const MARKER: &str = "JECODE_INTERNAL_SANDBOX_POLICY";

#[derive(Debug)]
pub struct Policy {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
    temporary: Option<PathBuf>,
    cargo_home: Option<PathBuf>,
    git_config: Option<PathBuf>,
    /// Real toolchain homes passed to the confined process by name.
    runtime_homes: Vec<(&'static str, PathBuf)>,
    /// Installed Node.js runtime locations. Empty when Node.js is absent.
    node_paths: Vec<PathBuf>,
    #[cfg(unix)]
    protected_home: Option<PathBuf>,
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
            temporary: None,
            cargo_home: None,
            git_config: None,
            runtime_homes: Vec::new(),
            node_paths: Vec::new(),
            #[cfg(unix)]
            protected_home: None,
        };
        if let Some(temporary) = temporary {
            let temporary = canonical_directory(temporary)?;
            policy.cargo_home = Some(temporary.join("cargo"));
            policy.git_config = Some(temporary.join("gitconfig"));
            policy.temporary = Some(temporary.clone());
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
        command.env(MARKER, self.native_policy()?);
        Ok(())
    }

    #[cfg(windows)]
    fn native_policy(&self) -> Result<OsString, String> {
        Ok(self.encode().into())
    }

    #[cfg(target_os = "linux")]
    fn native_policy(&self) -> Result<OsString, String> {
        use std::os::unix::ffi::OsStringExt;
        // Name resolution reads this file, and systems link it outside /etc.
        let resolver = std::fs::canonicalize("/etc/resolv.conf").ok();
        let rules = unix::landlock_rules(
            &self.read,
            &self.write,
            resolver.as_slice(),
            self.protected_home.as_deref(),
        )?;
        Ok(OsString::from_vec(rules))
    }

    #[cfg(target_os = "macos")]
    fn native_policy(&self) -> Result<OsString, String> {
        unix::seatbelt_profile(&self.read, &self.write, self.protected_home.as_deref())
            .map(Into::into)
    }

    /// A recursive workspace write grant cannot safely contain credentials
    /// or other saved conversations in Jecode's configuration directory.
    pub fn protect_session_home(&mut self, home: Option<&Path>) -> Result<(), String> {
        let Some(home) = home else { return Ok(()) };
        let home = canonical_directory(home)?;
        if self.write.iter().any(|root| home.starts_with(root)) {
            return Err("Jecode's configuration or other sessions are inside the working folder. Workspace commands cannot be confined here; choose a narrower working folder or /permissions full.".into());
        }
        #[cfg(unix)]
        {
            self.protected_home = Some(home);
        }
        Ok(())
    }

    /// Node version managers keep runtimes in the user profile, but a confined
    /// Windows process sees its own container as LOCALAPPDATA. Pass each manager's
    /// real home and grant it read-only, so shims resolve the selected runtime. On
    /// Windows, any installed runtime, including the first `node.exe` on PATH, also
    /// needs its drive roots enumerable; see `enumerable_roots`.
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
        let home = variable("HOME").map(PathBuf::from);
        let in_home = |relative: &str| {
            home.as_ref()
                .map(|home| home.join(relative))
                .filter(|path| path.is_dir())
        };
        let homes = [
            (
                "VOLTA_HOME",
                variable("VOLTA_HOME")
                    .map(PathBuf::from)
                    .or_else(|| local.map(|local| local.join("Volta")))
                    .or_else(|| in_home(".volta")),
            ),
            ("NVM_HOME", variable("NVM_HOME").map(PathBuf::from)),
            ("NVM_SYMLINK", variable("NVM_SYMLINK").map(PathBuf::from)),
            (
                "NVM_DIR",
                variable("NVM_DIR")
                    .map(PathBuf::from)
                    .or_else(|| in_home(".nvm")),
            ),
            (
                "FNM_DIR",
                variable("FNM_DIR")
                    .map(PathBuf::from)
                    .or_else(|| in_home(".local/share/fnm"))
                    .or_else(|| in_home("Library/Application Support/fnm")),
            ),
        ];
        let mut manager_paths = Vec::new();
        for (name, path) in homes {
            let Some(path) = path.filter(|path| path.is_absolute() && path.is_dir()) else {
                continue;
            };
            self.allow_existing_read_directory(&path, protected_home)?;
            manager_paths.push(path.clone());
            self.runtime_homes.push((name, path));
        }
        let node_in = |directory: &Path| {
            ["node.exe", "node"]
                .into_iter()
                .map(|name| directory.join(name))
                .find(|file| file.is_file())
        };
        if let Some(directory) = variable("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .find(|directory| directory.is_absolute() && node_in(directory).is_some())
        }) {
            self.add_node_path(&directory);
            // A Unix install keeps npm and global packages beside `bin`. Only those are
            // granted: a prefix such as `~/.local` also holds other programs' data, and
            // the home itself can look like one.
            if let Some(prefix) = std::fs::canonicalize(&directory)
                .ok()
                .and_then(|bin| bin.parent().map(Path::to_path_buf))
                .filter(|prefix| prefix.join("lib/node_modules").is_dir())
                && !home
                    .as_deref()
                    .and_then(|home| std::fs::canonicalize(home).ok())
                    .is_some_and(|home| home.starts_with(&prefix))
            {
                self.allow_existing_read_directory(&prefix.join("bin"), protected_home)?;
                self.allow_existing_read_directory(
                    &prefix.join("lib/node_modules"),
                    protected_home,
                )?;
            }
            // A file link can place node.exe on another drive even when its
            // PATH directory is not itself linked.
            if let Some(target) =
                node_in(&directory).and_then(|file| std::fs::canonicalize(file).ok())
                && let Some(parent) = target.parent()
            {
                self.add_node_path(parent);
            }
            // Version managers can keep the selected runtime away from the
            // executable on PATH. Their drives are needed only with Node.
            for path in manager_paths {
                self.add_node_path(&path);
            }
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
    /// file contents or writes. Windows lets a listing entry replace a grant
    /// for the same path, so a granted root stays out of the list, and the
    /// native adapter accepts only the canonical separator.
    #[cfg(any(windows, test))]
    fn enumerable_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        if self.node_paths.is_empty() {
            return roots;
        }
        let plain = |path: &Path| {
            PathBuf::from(
                crate::permissions::plain(path).replace('/', std::path::MAIN_SEPARATOR_STR),
            )
        };
        let granted: Vec<PathBuf> = self
            .read
            .iter()
            .chain(&self.write)
            .map(|path| plain(path))
            .collect();
        for path in self.write.iter().chain(&self.node_paths) {
            let Some(root) = path.ancestors().last().filter(|root| root.has_root()) else {
                continue;
            };
            let root = plain(root);
            if !granted
                .iter()
                .chain(&roots)
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
    // A Windows container gives each command its own application data. On Unix,
    // point the caches and settings programs write under the user's home at the
    // temporary area, so they work without access to the real ones.
    if cfg!(unix)
        && let Some(temporary) = &policy.temporary
    {
        for (name, folder) in [
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
            ("npm_config_cache", "npm"),
            ("CLANG_MODULE_CACHE_PATH", "clang-modules"),
        ] {
            if !explicit.iter().any(|(key, _)| key == name) {
                command.env(name, temporary.join(folder));
            }
        }
    }
    #[cfg(target_os = "macos")]
    apple::configure(command, explicit.iter().any(|(key, _)| key == "SDKROOT"));
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
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
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "LANG",
        "LANGUAGE",
        "TZ",
        "TERM",
        "COLORTERM",
        "DEVELOPER_DIR",
        "SDKROOT",
    ]
    .iter()
    .any(|allowed| key.eq_ignore_ascii_case(allowed))
        || key.starts_with("LC_")
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
mod node_tests;
#[cfg(test)]
mod tests;
#[cfg(all(test, unix))]
mod unix_escape_tests;
#[cfg(all(test, unix))]
mod unix_tests;
#[cfg(all(test, windows))]
mod windows_tests;
