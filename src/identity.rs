//! Runtime executable identity for diagnostics. The fingerprint is not an
//! authenticity check or a security token.

use crate::json::Value;
use std::fs::File;
use std::hash::Hasher;
use std::io::{self, Read};
use std::sync::OnceLock;

static EXECUTABLE: OnceLock<Option<File>> = OnceLock::new();

/// Keep the running binary open before its installed path can be replaced.
pub fn capture() {
    EXECUTABLE.get_or_init(open_running_executable);
}

fn open_running_executable() -> Option<File> {
    // procfs follows the loaded image even if its directory entry is gone.
    #[cfg(target_os = "linux")]
    if let Ok(file) = File::open("/proc/self/exe") {
        return Some(file);
    }

    std::env::current_exe()
        .ok()
        .and_then(|path| File::open(path).ok())
}

pub fn fingerprint() -> Option<&'static str> {
    static FINGERPRINT: OnceLock<Option<String>> = OnceLock::new();
    FINGERPRINT
        .get_or_init(|| {
            capture();
            hash_file(EXECUTABLE.get()?.as_ref()?).ok()
        })
        .as_deref()
}

fn hash_file(mut file: &File) -> io::Result<String> {
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

    #[cfg(unix)]
    #[test]
    fn first_export_after_executable_replacement_uses_running_binary() {
        use crate::test_support::Directory;
        use std::fs;
        use std::process::{Command, Stdio};
        use std::thread;
        use std::time::{Duration, Instant};

        let directory = Directory::new();
        let running = directory.path().join("running-jecode");
        let replacement = directory.path().join("replacement");
        fs::copy(std::env::current_exe().unwrap(), &running).unwrap();
        fs::write(&replacement, b"replacement binary").unwrap();
        let expected = hash_file(&File::open(&running).unwrap()).unwrap();
        let mut child = Command::new(&running)
            .args([
                "--exact",
                "identity::tests::replacement_child_reports_first_export",
                "--nocapture",
            ])
            .env("JECODE_IDENTITY_TEST_DIR", directory.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let ready = directory.path().join("ready");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited before capture"
            );
            assert!(
                Instant::now() < deadline,
                "child did not capture executable"
            );
            thread::sleep(Duration::from_millis(10));
        }

        fs::rename(&replacement, &running).unwrap();
        assert_ne!(hash_file(&File::open(&running).unwrap()).unwrap(), expected);
        fs::write(directory.path().join("continue"), b"").unwrap();
        assert!(
            child.wait().unwrap().success(),
            "child did not export identity"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("fingerprint")).unwrap(),
            expected
        );
    }

    #[cfg(unix)]
    #[test]
    fn replacement_child_reports_first_export() {
        use std::fs;
        use std::thread;
        use std::time::{Duration, Instant};

        let Some(directory) = std::env::var_os("JECODE_IDENTITY_TEST_DIR") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        capture();
        fs::write(directory.join("ready"), b"").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !directory.join("continue").exists() {
            assert!(
                Instant::now() < deadline,
                "parent did not replace executable"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let fingerprint = value()
            .get("executable_fingerprint")
            .and_then(Value::as_str)
            .unwrap()
            .to_owned();
        assert!(version_line().contains(&fingerprint));
        fs::write(directory.join("fingerprint"), fingerprint).unwrap();
    }
}
