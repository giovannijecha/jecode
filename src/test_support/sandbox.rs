//! Copy the small owned Rust probe into each granted workspace before launch.

use std::path::{Path, PathBuf};

pub const ROLE: &str = "JECODE_SANDBOX_PROBE_ROLE";
pub const ALLOWED: &str = "JECODE_SANDBOX_PROBE_ALLOWED";
pub const PRIVATE: &str = "JECODE_SANDBOX_PROBE_PRIVATE";
pub const ESCAPED: &str = "JECODE_SANDBOX_PROBE_ESCAPED";

pub fn executable(root: &Path) -> PathBuf {
    let fixture = root.join(if cfg!(windows) {
        "jecode-sandbox-probe.exe"
    } else {
        "jecode-sandbox-probe"
    });
    std::fs::write(
        &fixture,
        include_bytes!(concat!(env!("OUT_DIR"), "/sandbox-probe-fixture")),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    fixture
}
