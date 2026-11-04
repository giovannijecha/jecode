//! An owned stdio language server compiled into an isolated test directory.
use super::Profile;
use crate::test_support::Directory;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

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
    /// Whether server process `pid` is still running. Each one holds a lock on
    /// its own file until it ends, whether it exits or is killed.
    pub fn running(&self, pid: u32) -> bool {
        let mut path = self.marker().into_os_string();
        path.push(format!("-{pid}.lock"));
        let file = fs::File::open(path).expect("fixture server lock file");
        match file.try_lock() {
            Ok(()) => false,
            Err(fs::TryLockError::WouldBlock) => true,
            Err(fs::TryLockError::Error(error)) => panic!("fixture server lock: {error}"),
        }
    }
    /// Waits for server process `pid` to end, within a bound that covers the
    /// shutdown handshake and a kill during a loaded parallel run.
    #[track_caller]
    pub fn wait_stopped(&self, pid: u32, failure: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.running(pid) {
            assert!(Instant::now() < deadline, "{failure}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    /// Lets the requests the fixture server holds (column 10) reply.
    pub fn release(&self) {
        let mut path = self.marker().into_os_string();
        path.push("-release");
        fs::write(path, "").unwrap();
    }
    pub fn profile(&self) -> Profile {
        let mut profile = Profile::draft_guided_rust();
        profile.executable = self.executable.to_string_lossy().into_owned();
        profile.arguments = vec![self.marker().to_string_lossy().into_owned()];
        profile.root_override = Some(self.workspace().to_path_buf());
        profile
    }
}
