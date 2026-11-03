//! An owned stdio language server compiled into an isolated test directory.
use super::Profile;
use crate::test_support::Directory;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Fixture {
    directory: Directory,
    executable: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = Directory::new();
        let source = directory.path().join("server.rs");
        let executable = directory
            .path()
            .join(format!("server{}", std::env::consts::EXE_SUFFIX));
        fs::write(&source, include_str!("fixture_server.rs")).unwrap();
        let mut command = Command::new("rustc");
        command
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&executable);
        if let Ok(linker) = std::env::var("CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER") {
            command.arg("-C").arg(format!("linker={linker}"));
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "Fixture server compile failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::write(directory.path().join("sample.rs"), "pub fn sample() {}\n").unwrap();
        Self {
            directory,
            executable,
        }
    }
    pub fn workspace(&self) -> &Path {
        self.directory.path()
    }
    pub fn sample(&self) -> PathBuf {
        self.directory.path().join("sample.rs")
    }
    pub fn marker(&self) -> PathBuf {
        self.directory.path().join("started")
    }
    pub fn profile(&self) -> Profile {
        let mut profile = Profile::draft_guided_rust();
        profile.executable = self.executable.to_string_lossy().into_owned();
        profile.arguments = vec![self.marker().to_string_lossy().into_owned()];
        profile.root_override = Some(self.workspace().to_path_buf());
        profile
    }
}
