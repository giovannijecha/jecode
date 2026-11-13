//! Operating-system steps for replacing or removing a running executable.
//! Windows locks a running program's file but lets it be renamed; the
//! rename frees its name, and a detached Windows PowerShell removes the old
//! file after the process exits. Unix replaces and unlinks running files.

use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Stdio;

/// A running executable moved out of the way, restorable until committed.
pub struct Moved {
    pub from: PathBuf,
    pub to: PathBuf,
}

impl Moved {
    pub fn restore(&self) -> Result<(), String> {
        std::fs::rename(&self.to, &self.from).map_err(|error| {
            format!(
                "Could not restore {} from {}: {error}",
                self.from.display(),
                self.to.display()
            )
        })
    }
}

fn unique(directory: &Path, stem: &str) -> PathBuf {
    let extension = if cfg!(windows) { ".exe" } else { "" };
    directory.join(format!(
        "{stem}-{}-{}{extension}",
        std::process::id(),
        crate::sessions::now()
    ))
}

/// Moves `binary` to the first candidate directory on the same volume.
pub fn move_aside(binary: &Path, directories: &[PathBuf], stem: &str) -> Result<Moved, String> {
    let mut last = None;
    for directory in directories {
        if !directory.is_dir() {
            continue;
        }
        let to = unique(directory, stem);
        match std::fs::rename(binary, &to) {
            Ok(()) => {
                return Ok(Moved {
                    from: binary.to_path_buf(),
                    to,
                });
            }
            Err(error) => last = Some(error),
        }
    }
    Err(format!(
        "Could not move the running executable {} aside{}",
        binary.display(),
        last.map_or_else(String::new, |error| format!(": {error}"))
    ))
}

/// Removes `paths` now when possible; on Windows a file still in use, such
/// as this process's own image, is removed by a detached helper after this
/// process exits. Returns the paths whose removal was deferred.
pub fn remove_later(paths: &[PathBuf], directories: &[PathBuf]) -> Vec<PathBuf> {
    let mut pending = Vec::new();
    for path in paths {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => pending.push(path.clone()),
        }
    }
    for directory in directories {
        let _ = std::fs::remove_dir(directory);
    }
    #[cfg(windows)]
    if !pending.is_empty() || directories.iter().any(|directory| directory.exists()) {
        let mut listed = pending.clone();
        listed.extend(
            directories
                .iter()
                .filter(|directory| directory.exists())
                .cloned(),
        );
        if spawn_cleanup(&listed).is_err() {
            return pending;
        }
    }
    pending
}

/// Waits for this process to end, then removes files and empty directories.
#[cfg(windows)]
fn spawn_cleanup(paths: &[PathBuf]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let script = r#"$ErrorActionPreference = 'SilentlyContinue'
Wait-Process -Id ([int]$env:JECODE_CLEANUP_PID)
$paths = $env:JECODE_CLEANUP_PATHS -split "`n" | Where-Object { $_ }
for ($attempt = 0; $attempt -lt 20; $attempt++) {
    $left = @()
    foreach ($path in $paths) {
        if (Test-Path -LiteralPath $path -PathType Leaf) { Remove-Item -LiteralPath $path -Force }
        elseif (Test-Path -LiteralPath $path -PathType Container) {
            if (-not (Get-ChildItem -LiteralPath $path -Force)) { Remove-Item -LiteralPath $path -Force }
        }
        if (Test-Path -LiteralPath $path) { $left += $path }
    }
    if ($left.Count -eq 0) { break }
    Start-Sleep -Milliseconds 500
}"#;
    let joined = paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    let start = |flags: u32| {
        let mut command = crate::native::powershell();
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                script,
            ])
            .env("JECODE_CLEANUP_PID", std::process::id().to_string())
            .env("JECODE_CLEANUP_PATHS", &joined)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags);
        crate::auth::isolate_environment(&mut command).spawn()
    };
    start(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB)
        .or_else(|_| start(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW))
        .map(|_| ())
        .map_err(|error| format!("Could not schedule cleanup: {error}"))
}

/// Entries of a `PATH` value that name `directory`, compared as INSTALL.ps1
/// adds it: environment references expanded, quotes and a trailing
/// separator ignored, case-insensitively. Other entries keep their text.
/// Only Windows installers change `PATH`.
#[cfg(any(windows, test))]
pub fn without_directory(path: &str, directory: &Path) -> (String, usize) {
    let target = normalize(&directory.to_string_lossy());
    let mut removed = 0;
    let kept: Vec<&str> = path
        .split(';')
        .filter(|entry| {
            let matches = !entry.trim().is_empty() && normalize(&expand(entry)) == target;
            if matches {
                removed += 1;
            }
            !matches
        })
        .collect();
    (kept.join(";"), removed)
}

#[cfg(any(windows, test))]
fn normalize(entry: &str) -> String {
    entry
        .trim()
        .trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

#[cfg(any(windows, test))]
fn expand(entry: &str) -> String {
    let mut output = String::new();
    let mut rest = entry;
    while let Some(start) = rest.find('%') {
        output.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) if !name.is_empty() => output.push_str(&value),
                    _ => {
                        output.push('%');
                        output.push_str(name);
                        output.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                output.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    output.push_str(rest);
    output
}

/// Removes `directory` from the user `PATH` in the registry, keeping the
/// value's type (`REG_EXPAND_SZ` stays expandable), then announces the
/// change so new terminals see it. Returns how many entries were removed.
#[cfg(windows)]
pub fn remove_user_path(directory: &Path, key: &str) -> Result<usize, String> {
    let read = r#"$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($env:JECODE_PATH_KEY)
if ($null -eq $key) { exit 0 }
$value = $key.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
if ($null -eq $value) { exit 0 }
[Console]::Out.Write($key.GetValueKind('Path').ToString() + "`n" + [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($value)))"#;
    let output = powershell(read, &[("JECODE_PATH_KEY", key)])?;
    let Some((kind, encoded)) = output.split_once('\n') else {
        return Ok(0);
    };
    let value = crate::attachments::base64::decode(encoded.trim())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or("Could not read the user PATH")?;
    let (next, removed) = without_directory(&value, directory);
    if removed == 0 {
        return Ok(0);
    }
    let write = r#"$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($env:JECODE_PATH_KEY, $true)
$value = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($env:JECODE_PATH_VALUE))
$kind = [Microsoft.Win32.RegistryValueKind]$env:JECODE_PATH_KIND
$key.SetValue('Path', $value, $kind)
$key.Close()
if ($env:JECODE_PATH_KEY -eq 'Environment') {
    # .NET announces WM_SETTINGCHANGE after a user environment change.
    [Environment]::SetEnvironmentVariable('JECODE_PATH_REFRESH', $null, 'User')
}
'ok'"#;
    let encoded = crate::attachments::base64::encode(next.as_bytes());
    let result = powershell(
        write,
        &[
            ("JECODE_PATH_KEY", key),
            ("JECODE_PATH_VALUE", &encoded),
            ("JECODE_PATH_KIND", kind.trim()),
        ],
    )?;
    if result.trim() != "ok" {
        return Err("Could not update the user PATH".into());
    }
    Ok(removed)
}

#[cfg(windows)]
fn powershell(script: &str, environment: &[(&str, &str)]) -> Result<String, String> {
    let mut command = crate::native::powershell();
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::null());
    for (name, value) in environment {
        command.env(name, value);
    }
    let output = crate::auth::isolate_environment(&mut command)
        .output()
        .map_err(|error| format!("Could not start Windows PowerShell: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Windows PowerShell failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| "Windows PowerShell output was not UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_removal_keeps_other_entries_verbatim() {
        let directory = Path::new(r"C:\Users\Example\AppData\Local\Jecode\bin");
        let (kept, removed) = without_directory(
            r#"C:\Tools;"C:\Users\example\AppData\Local\Jecode\bin\";;%SystemRoot%\system32;C:\Users\Example\AppData\Local\Jecode\binaries"#,
            directory,
        );
        assert_eq!(removed, 1);
        assert_eq!(
            kept,
            r"C:\Tools;;%SystemRoot%\system32;C:\Users\Example\AppData\Local\Jecode\binaries"
        );
        let (unchanged, none) = without_directory(r"C:\Tools", directory);
        assert_eq!((unchanged.as_str(), none), (r"C:\Tools", 0));
    }

    #[test]
    fn environment_references_expand_before_comparison() {
        // SAFETY-free: read-only use of an existing variable.
        let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        else {
            return;
        };
        let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let directory = PathBuf::from(&home).join("Jecode").join("bin");
        let entry = format!(
            "%{name}%{}Jecode{}bin",
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        );
        assert_eq!(without_directory(&entry, &directory).1, 1);
        assert_eq!(
            expand("%JECODE_SURELY_UNSET_VARIABLE%x"),
            "%JECODE_SURELY_UNSET_VARIABLE%x"
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "writes a disposable key under HKCU\\Software"]
    fn registry_path_removal_keeps_the_value_kind_and_other_entries() {
        let key = format!("Software\\JecodeTest-{}\\Environment", std::process::id());
        let directory = Path::new(r"C:\Disposable\Jecode\bin");
        let setup = r#"$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($env:JECODE_PATH_KEY)
$key.SetValue('Path', '%SystemRoot%\system32;C:\Disposable\Jecode\bin;C:\Tools', [Microsoft.Win32.RegistryValueKind]::ExpandString)
$key.Close()"#;
        powershell(setup, &[("JECODE_PATH_KEY", &key)]).unwrap();
        let removed = remove_user_path(directory, &key);
        let read = r#"$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($env:JECODE_PATH_KEY)
$key.GetValueKind('Path').ToString() + '|' + $key.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)"#;
        let after = powershell(read, &[("JECODE_PATH_KEY", &key)]);
        let parent = key.trim_end_matches("\\Environment").to_owned();
        powershell(
            "[Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($env:JECODE_PATH_KEY)",
            &[("JECODE_PATH_KEY", &parent)],
        )
        .unwrap();
        assert_eq!(removed.unwrap(), 1);
        assert_eq!(
            after.unwrap().trim(),
            r"ExpandString|%SystemRoot%\system32;C:\Tools"
        );
    }

    #[test]
    fn moving_aside_restores_and_reports_missing_directories() {
        let directory = crate::test_support::Directory::new();
        let binary = directory.path().join("program");
        std::fs::write(&binary, b"old").unwrap();
        let elsewhere = directory.path().join("aside");
        std::fs::create_dir(&elsewhere).unwrap();
        let moved = move_aside(
            &binary,
            &[directory.path().join("missing"), elsewhere.clone()],
            ".jecode-running",
        )
        .unwrap();
        assert!(!binary.exists() && moved.to.starts_with(&elsewhere));
        moved.restore().unwrap();
        assert_eq!(std::fs::read(&binary).unwrap(), b"old");
        assert!(move_aside(&binary, &[directory.path().join("missing")], "x").is_err());
        let pending = remove_later(
            std::slice::from_ref(&binary),
            std::slice::from_ref(&elsewhere),
        );
        assert!(pending.is_empty() && !binary.exists() && !elsewhere.exists());
    }
}
