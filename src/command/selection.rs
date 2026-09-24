//! Resolve one command shell for a session; a configured executable never falls back.
use std::io;
#[cfg(windows)]
use std::{
    os::windows::{ffi::OsStrExt, process::CommandExt},
    path::{Component, Path, PathBuf, Prefix},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default)]
pub(crate) enum Shell {
    #[cfg(windows)]
    #[default]
    WindowsPowerShell,
    #[cfg(windows)]
    PowerShell7 {
        executable: PathBuf,
        version: String,
        bracket_cwd: BracketCwd,
    },
    #[cfg(not(windows))]
    #[default]
    Sh,
}
#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BracketCwd {
    // This state exists only while the fixed probe runs through the real runner.
    Probing,
    Supported,
    Incompatible,
    Inconclusive(ProbeIssue),
}
#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProbeIssue {
    Fixture,
    Launch,
    Timeout,
    Output,
    Malformed,
    Cleanup,
}
#[cfg(windows)]
impl ProbeIssue {
    fn label(self) -> &'static str {
        match self {
            Self::Fixture => "temporary fixture unavailable",
            Self::Launch => "probe command could not start or finish",
            Self::Timeout => "probe command timed out",
            Self::Output => "probe output exceeded its limit or failed",
            Self::Malformed => "probe output was incomplete or malformed",
            Self::Cleanup => "probe cleanup could not be confirmed",
        }
    }
}
impl Shell {
    pub(crate) fn configured(
        setting: Option<&str>,
        selected_directory: Option<&std::path::Path>,
    ) -> io::Result<Self> {
        #[cfg(not(windows))]
        {
            let _ = (setting, selected_directory);
            Ok(Self::Sh)
        }
        #[cfg(windows)]
        {
            let Some(setting) = setting else {
                return Ok(Self::WindowsPowerShell);
            };
            let invalid = || {
                crate::state::settings::shell_config_error(
                    "windows_powershell_executable must name an absolute local PowerShell 7 executable in ~/.jecode/v1/settings.json; remove it to use Windows PowerShell 5.1",
                )
            };
            let requested = Path::new(setting);
            let drive = matches!(requested.components().next(), Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)));
            if !requested.is_absolute()
                || !drive
                || setting
                    .chars()
                    .any(|c| c.is_control() || super::capture::invisible(c))
            {
                return Err(invalid());
            }
            let executable = requested.canonicalize().map_err(|_| {
                crate::state::settings::shell_config_error(
                    "configured windows_powershell_executable is unavailable; install PowerShell 7 at that path or remove the setting to use Windows PowerShell 5.1",
                )
            })?;
            if !executable
                .metadata()
                .is_ok_and(|metadata| metadata.is_file())
            {
                return Err(invalid());
            }
            let version = probe_version(&executable)?;
            let mut shell = Self::PowerShell7 {
                executable,
                version,
                bracket_cwd: BracketCwd::Probing,
            };
            let result = super::probe::check(&shell, selected_directory);
            if let Self::PowerShell7 { bracket_cwd, .. } = &mut shell {
                *bracket_cwd = result;
            }
            Ok(shell)
        }
    }
    pub(crate) fn label(&self) -> String {
        match self {
            #[cfg(windows)]
            Self::WindowsPowerShell => "Windows PowerShell 5.1 / no profile".into(),
            #[cfg(windows)]
            Self::PowerShell7 {
                executable,
                version,
                bracket_cwd,
            } => {
                format!(
                    "PowerShell {version} / no profile / {} / bracketed cwd: {}",
                    executable.display(),
                    match bracket_cwd {
                        BracketCwd::Probing => "checking".into(),
                        BracketCwd::Supported => "supported by session probe".into(),
                        BracketCwd::Incompatible => "incompatible with session probe".into(),
                        BracketCwd::Inconclusive(issue) =>
                            format!("unverified ({})", issue.label()),
                    }
                )
            }
            #[cfg(not(windows))]
            Self::Sh => "/bin/sh / non-interactive".into(),
        }
    }
    pub(crate) fn limitation(&self) -> &'static str {
        match self {
            #[cfg(windows)]
            Self::WindowsPowerShell => {
                "Commands cannot start in directories containing [ or ] with this shell."
            }
            #[cfg(windows)]
            Self::PowerShell7 {
                bracket_cwd: BracketCwd::Incompatible,
                ..
            } => {
                "The configured PowerShell 7 failed the bracketed-directory capability check. Commands cannot start in directories containing [ or ] with this shell."
            }
            #[cfg(windows)]
            Self::PowerShell7 {
                bracket_cwd: BracketCwd::Inconclusive(_),
                ..
            } => {
                "Bracketed-directory capability could not be verified for the configured PowerShell 7; commands cannot start in directories containing [ or ] until a new session verifies it."
            }
            _ => "",
        }
    }
    #[cfg(windows)]
    pub(crate) fn cwd_error(&self, path: &Path) -> Option<&'static str> {
        let brackets = path
            .as_os_str()
            .encode_wide()
            .any(|unit| unit == b'[' as u16 || unit == b']' as u16);
        if !brackets {
            return None;
        }
        match self {
            Self::WindowsPowerShell => Some(
                "Windows PowerShell 5.1 cannot safely run commands from a directory containing [ or ]; set windows_powershell_executable to a verified PowerShell 7 pwsh.exe in ~/.jecode/v1/settings.json and start a new session",
            ),
            Self::PowerShell7 {
                bracket_cwd: BracketCwd::Incompatible,
                ..
            } => Some(
                "configured PowerShell 7 failed the bracketed-directory capability probe; select a compatible PowerShell 7 executable in ~/.jecode/v1/settings.json and start a new session",
            ),
            Self::PowerShell7 {
                bracket_cwd: BracketCwd::Inconclusive(ProbeIssue::Fixture),
                ..
            } => Some(
                "bracketed-directory capability could not be checked because the isolated temporary fixture was unavailable; check the temporary directory and start a new session",
            ),
            Self::PowerShell7 {
                bracket_cwd: BracketCwd::Inconclusive(ProbeIssue::Launch),
                ..
            } => Some(
                "bracketed-directory capability could not be checked because the configured PowerShell 7 probe could not start or finish; check the executable and start a new session",
            ),
            Self::PowerShell7 {
                bracket_cwd: BracketCwd::Inconclusive(ProbeIssue::Timeout),
                ..
            } => Some(
                "bracketed-directory capability could not be checked because the configured PowerShell 7 probe timed out; start a new session to retry",
            ),
            Self::PowerShell7 {
                bracket_cwd: BracketCwd::Inconclusive(ProbeIssue::Output),
                ..
            } => Some(
                "bracketed-directory capability could not be checked because probe output failed or exceeded its limit; start a new session to retry",
            ),
            Self::PowerShell7 {
                bracket_cwd: BracketCwd::Inconclusive(ProbeIssue::Malformed),
                ..
            } => Some(
                "bracketed-directory capability could not be checked because the configured PowerShell 7 probe returned malformed output; start a new session to retry",
            ),
            Self::PowerShell7 {
                bracket_cwd: BracketCwd::Inconclusive(ProbeIssue::Cleanup),
                ..
            } => Some(
                "bracketed-directory capability could not be checked because probe cleanup could not be confirmed; inspect the temporary directory before retrying",
            ),
            Self::PowerShell7 {
                bracket_cwd: BracketCwd::Probing,
                ..
            } => None,
            Self::PowerShell7 {
                bracket_cwd: BracketCwd::Supported,
                ..
            } => None,
        }
    }
}

#[cfg(windows)]
fn probe_version(executable: &Path) -> io::Result<String> {
    let unavailable = || {
        crate::state::settings::shell_config_error(
            "configured windows_powershell_executable could not start or identify PowerShell 7; check the path and executable, or remove the setting to use Windows PowerShell 5.1",
        )
    };
    let mut child = Command::new(executable)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::Out.Write($PSVersionTable.PSVersion.ToString())",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|_| unavailable())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unavailable());
            }
        }
    };
    let output = child.wait_with_output().map_err(|_| unavailable())?;
    if !status.success() || !output.stderr.is_empty() || output.stdout.len() > 32 {
        return Err(unavailable());
    }
    let version = std::str::from_utf8(&output.stdout)
        .map_err(|_| unavailable())?
        .trim();
    let parts = version.split('.').collect::<Vec<_>>();
    if !(2..=4).contains(&parts.len())
        || parts[0] != "7"
        || parts
            .iter()
            .any(|part| part.is_empty() || part.parse::<u32>().is_err())
    {
        return Err(crate::state::settings::shell_config_error(
            "configured windows_powershell_executable did not report PowerShell 7; select pwsh.exe or remove the setting to use Windows PowerShell 5.1",
        ));
    }
    Ok(version.into())
}
