//! A Rust fixture for Windows Process Security Environment tests.
//! The test executable is copied inside the granted workspace before launch.

use crate::json::Value;
use std::path::{Path, PathBuf};

pub const ROLE: &str = "JECODE_SANDBOX_PROBE_ROLE";
pub const ALLOWED: &str = "JECODE_SANDBOX_PROBE_ALLOWED";
pub const PRIVATE: &str = "JECODE_SANDBOX_PROBE_PRIVATE";
pub const ESCAPED: &str = "JECODE_SANDBOX_PROBE_ESCAPED";

pub fn executable(root: &Path) -> PathBuf {
    let fixture = root.join("jecode-sandbox-probe.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &fixture).unwrap();
    fixture
}

pub fn args() -> Value {
    Value::Array(
        [
            "test_support::sandbox::probe",
            "--exact",
            "--ignored",
            "--nocapture",
        ]
        .into_iter()
        .map(Value::string)
        .collect(),
    )
}

#[test]
#[ignore = "launched by the Windows confinement tests as a separate process"]
fn probe() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    match role.as_str() {
        "environment" => {
            assert_eq!(std::env::var("JECODE_TEST_ENV").as_deref(), Ok("sentinel"));
            assert!(std::env::var_os("JECODE_SECRET_CANARY").is_none());
            assert!(std::env::var_os("GH_TOKEN").is_none());
            println!("WORKSPACE_NATIVE:sentinel:");
        }
        "write" => {
            let root = std::env::var_os(ALLOWED).unwrap();
            let nested = PathBuf::from(root).join("nested");
            std::fs::create_dir(&nested).unwrap();
            let file = nested.join("item.txt");
            std::fs::write(&file, "written").unwrap();
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "written");
            println!("CREATED:written");
            std::fs::remove_file(file).unwrap();
            std::fs::remove_dir(nested).unwrap();
        }
        "parent" => {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "test_support::sandbox::probe",
                    "--exact",
                    "--ignored",
                    "--nocapture",
                ])
                .env(ROLE, "child")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            print!("{}", String::from_utf8_lossy(&output.stdout));
        }
        "child" => {
            let allowed = PathBuf::from(std::env::var_os(ALLOWED).unwrap());
            let private = PathBuf::from(std::env::var_os(PRIVATE).unwrap());
            let escaped = PathBuf::from(std::env::var_os(ESCAPED).unwrap());
            assert_eq!(std::fs::read_to_string(&allowed).unwrap(), "allowed");
            println!("INSIDE:allowed");
            assert_eq!(
                std::fs::read_to_string(private).unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
            println!("OUTSIDE_ERROR:PermissionDenied");
            assert_eq!(
                std::fs::write(escaped, "escaped").unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
            println!("WRITE_ERROR:PermissionDenied");
            let nested = allowed.parent().unwrap().join("nested");
            std::fs::create_dir(&nested).unwrap();
            let file = nested.join("item.txt");
            std::fs::write(&file, "created").unwrap();
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "created");
            println!("CREATED:created");
            std::fs::remove_file(file).unwrap();
            std::fs::remove_dir(nested).unwrap();
        }
        other => panic!("Unknown sandbox probe role: {other}"),
    }
}
