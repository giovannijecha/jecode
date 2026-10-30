//! Copy the small owned Rust probe into each granted workspace before launch.

use std::path::{Path, PathBuf};

pub const ROLE: &str = "JECODE_SANDBOX_PROBE_ROLE";
pub const ALLOWED: &str = "JECODE_SANDBOX_PROBE_ALLOWED";
pub const PRIVATE: &str = "JECODE_SANDBOX_PROBE_PRIVATE";
pub const ESCAPED: &str = "JECODE_SANDBOX_PROBE_ESCAPED";

pub fn executable(root: &Path) -> PathBuf {
    let fixture = root.join("jecode-sandbox-probe.exe");
    std::fs::write(
        &fixture,
        include_bytes!(concat!(env!("OUT_DIR"), "/jecode-sandbox-probe.exe")),
    )
    .unwrap();
    fixture
}
