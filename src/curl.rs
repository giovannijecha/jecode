use std::ffi::OsString;

/// Honor an explicit curl override, then prefer the Windows system executable.
pub(crate) fn executable() -> OsString {
    if let Some(path) = std::env::var_os("JECODE_CURL").filter(|value| !value.is_empty()) {
        return path;
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        let path = std::path::PathBuf::from(root).join("System32/curl.exe");
        if path.is_file() {
            return path.into_os_string();
        }
    }
    "curl".into()
}
