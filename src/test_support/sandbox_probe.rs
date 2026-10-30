//! Standalone native fixture for Windows Process Security Environment tests.

use std::env;
use std::path::PathBuf;
use std::process::Command;

const ROLE: &str = "JECODE_SANDBOX_PROBE_ROLE";
const ALLOWED: &str = "JECODE_SANDBOX_PROBE_ALLOWED";
const PRIVATE: &str = "JECODE_SANDBOX_PROBE_PRIVATE";
const ESCAPED: &str = "JECODE_SANDBOX_PROBE_ESCAPED";

fn main() {
    match env::var(ROLE).expect("Sandbox probe role").as_str() {
        "environment" => {
            assert_eq!(env::var("JECODE_TEST_ENV").as_deref(), Ok("sentinel"));
            assert!(env::var_os("JECODE_SECRET_CANARY").is_none());
            assert!(env::var_os("GH_TOKEN").is_none());
            println!("WORKSPACE_NATIVE:sentinel:");
        }
        "write" => {
            let root = PathBuf::from(env::var_os(ALLOWED).unwrap());
            let nested = root.join("nested");
            std::fs::create_dir(&nested).unwrap();
            let file = nested.join("item.txt");
            std::fs::write(&file, "written").unwrap();
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "written");
            println!("CREATED:written");
            std::fs::remove_file(file).unwrap();
            std::fs::remove_dir(nested).unwrap();
        }
        "parent" => {
            let output = Command::new(env::current_exe().unwrap())
                .env(ROLE, "child")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            print!("{}", String::from_utf8_lossy(&output.stdout));
        }
        "child" => {
            let allowed = PathBuf::from(env::var_os(ALLOWED).unwrap());
            let private = PathBuf::from(env::var_os(PRIVATE).unwrap());
            let escaped = PathBuf::from(env::var_os(ESCAPED).unwrap());
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
