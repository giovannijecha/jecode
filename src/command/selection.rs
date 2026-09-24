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
    },
    #[cfg(not(windows))]
    #[default]
    Sh,
}
impl Shell {
    pub(crate) fn configured(setting: Option<&str>) -> io::Result<Self> {
        #[cfg(not(windows))]
        {
            let _ = setting;
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
            Ok(Self::PowerShell7 {
                executable,
                version,
            })
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
            } => {
                format!(
                    "PowerShell {version} / no profile / {}",
                    executable.display()
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
            Self::PowerShell7 { version, .. } if version != "7.6.6" => {
                "Commands cannot start in directories containing [ or ] with this unverified PowerShell 7 version."
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
            Self::PowerShell7 { version, .. } if version != "7.6.6" => Some(
                "this PowerShell 7 version has not been verified for a starting directory containing [ or ]; PowerShell 7.6.6 is verified",
            ),
            Self::PowerShell7 { .. } => None,
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
