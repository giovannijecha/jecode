//! Runtime executable identity for diagnostics. The fingerprint is not an
//! authenticity check or a security token.

use crate::json::Value;
use std::fs::File;
use std::hash::Hasher;
use std::io::{self, Read};
use std::path::Path;
use std::sync::OnceLock;

pub fn fingerprint() -> Option<&'static str> {
    static FINGERPRINT: OnceLock<Option<String>> = OnceLock::new();
    FINGERPRINT
        .get_or_init(|| {
            let executable = std::env::current_exe().ok()?;
            hash_file(&executable).ok()
        })
        .as_deref()
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.write(&buffer[..count]);
    }
    Ok(format!("{:016x}", hash.finish()))
}

fn profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

pub fn version_line() -> String {
    format!(
        "jecode {} ({}-{}-{}, exe {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        profile(),
        fingerprint().unwrap_or("unavailable")
    )
}

pub fn value() -> Value {
    Value::object([
        ("os", Value::string(std::env::consts::OS)),
        ("arch", Value::string(std::env::consts::ARCH)),
        ("profile", Value::string(profile())),
        (
            "executable_fingerprint",
            fingerprint().map_or(Value::Null, Value::string),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_and_export_share_the_current_executable_identity() {
        let fingerprint = fingerprint().expect("test executable is readable");
        assert_eq!(fingerprint.len(), 16);
        assert!(fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(version_line().contains(fingerprint));
        assert_eq!(
            value().get("executable_fingerprint"),
            Some(&Value::string(fingerprint))
        );
    }
}
