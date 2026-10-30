//! Owned adapters embedded at build time. Runtime needs OS APIs, not compilers.
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

include!(concat!(env!("OUT_DIR"), "/native_assets.rs"));

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Windows PowerShell is an OS component. Resolve it independently of PATH.
#[cfg(not(target_os = "macos"))]
pub(crate) fn powershell() -> std::process::Command {
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        return std::process::Command::new(
            PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        );
    }
    // WSL's Windows interop supplies this system command through its OS PATH.
    std::process::Command::new("powershell.exe")
}

/// Fully verified once per process and adapter; later calls recheck only the
/// cached entry's type and length, and fully reinstall if those changed.
pub(crate) fn helper(name: &str) -> Result<PathBuf, String> {
    static VERIFIED: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    let bytes = asset(name).ok_or_else(|| format!("No {name} adapter for this platform"))?;
    let verified = VERIFIED.get_or_init(Mutex::default);
    let cached = verified
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(name)
        .cloned();
    if let Some(path) = cached
        && plausible(&path, bytes.len())
    {
        return Ok(path);
    }
    let path = materialize(&root()?, name, bytes)?;
    verified
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(name.to_string(), path.clone());
    Ok(path)
}

fn plausible(path: &Path, length: usize) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        #[cfg(windows)]
        let redirected = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let redirected = false;
        metadata.is_file() && !redirected && metadata.len() == length as u64
    })
}

fn root() -> Result<PathBuf, String> {
    #[cfg(test)]
    return Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!(".tmp/native-tests-{}", std::process::id())));
    #[cfg(not(test))]
    {
        let store = crate::config::Store::discover()?;
        Ok(store
            .path()
            .parent()
            .ok_or("Could not locate Jecode storage")?
            .join("cache/native"))
    }
}

fn materialize(root: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("Invalid owned adapter name".into());
    }
    let identity = crate::permissions::identity::sha256_hex(bytes)?;
    let extension = if cfg!(windows) { ".exe" } else { "" };
    let path = root.join(format!("{name}-{identity}{extension}"));
    fs::create_dir_all(root)
        .map_err(|error| format!("Could not prepare native adapters: {error}"))?;
    if matches_asset(&path, bytes)? {
        return Ok(path);
    }
    let (temporary, mut file) = reserve_stage(root, &NEXT)?;
    let result = (|| {
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("Could not write native adapter: {error}"))?;
        drop(file);
        match fs::rename(&temporary, &path) {
            Ok(()) => {}
            Err(_) if matches_asset(&path, bytes)? => {}
            Err(error) => return Err(format!("Could not install native adapter: {error}")),
        }
        if !matches_asset(&path, bytes)? {
            return Err("Native adapter verification failed".into());
        }
        Ok(path)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn reserve_stage(root: &Path, counter: &AtomicU64) -> Result<(PathBuf, fs::File), String> {
    for _ in 0..64 {
        let path = root.join(format!(
            ".stage-{}-{}",
            std::process::id(),
            counter.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o700);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not stage native adapter: {error}")),
        }
    }
    Err("Native adapter staging names are already occupied".into())
}

fn matches_asset(path: &Path, bytes: &[u8]) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Could not inspect native adapter: {error}")),
    };
    let redirected = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let redirected = {
        use std::os::windows::fs::MetadataExt;
        redirected || metadata.file_attributes() & 0x400 != 0
    };
    if redirected || !metadata.is_file() || metadata.len() != bytes.len() as u64 {
        return Err("Native adapter cache contains an unexpected entry".into());
    }
    let actual =
        fs::read(path).map_err(|error| format!("Could not verify native adapter: {error}"))?;
    if actual != bytes {
        return Err("Native adapter cache differs from the embedded adapter".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("Native adapter cache entry is not executable".into());
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn extracts_exact_embedded_bytes_and_rejects_changed_cache() {
        let directory = Directory::new();
        let bytes = b"owned adapter bytes";
        let path = materialize(directory.path(), "fixture", bytes).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            materialize(directory.path(), "fixture", bytes).unwrap(),
            path
        );
        fs::write(&path, b"changed executable!").unwrap();
        assert!(materialize(directory.path(), "fixture", bytes).is_err());
    }

    #[test]
    fn a_stale_stage_is_preserved_and_a_fresh_name_is_reserved() {
        let directory = Directory::new();
        let occupied = directory
            .path()
            .join(format!(".stage-{}-0", std::process::id()));
        fs::write(&occupied, b"unrelated data").unwrap();
        let (path, file) = reserve_stage(directory.path(), &AtomicU64::new(0)).unwrap();
        assert_ne!(path, occupied);
        drop(file);
        assert_eq!(fs::read(&occupied).unwrap(), b"unrelated data");
        assert!(path.is_file());
    }

    #[test]
    fn bundled_adapter_runs_without_compilation() {
        let name = if cfg!(windows) {
            "checkpoint_space"
        } else {
            "process_unix"
        };
        let path = helper(name).unwrap();
        let mut command = std::process::Command::new(path);
        if cfg!(windows) {
            command.arg(std::env::temp_dir());
        } else {
            command.args(["space", "/"]);
        }
        command.env("PATH", "");
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .trim()
                .parse::<u64>()
                .is_ok()
        );
    }
}

#[cfg(test)]
#[path = "native/independence_tests.rs"]
mod independence_tests;
