use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

pub struct Fixture(pub PathBuf);
impl Fixture {
    #[allow(dead_code)]
    pub fn home(&self) -> PathBuf {
        let home = self.0.with_extension("home");
        fs::create_dir_all(&home).unwrap();
        home
    }
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/workspace-tests");
        fs::create_dir_all(&base).unwrap();
        let path = base.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub fn write(&self, path: &str, text: impl AsRef<[u8]>) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    #[allow(dead_code)]
    pub fn unsupported_host_filesystem(&self, error: &str) -> bool {
        cfg!(target_os = "linux")
            && self.0.starts_with("/mnt/c/")
            && error.contains("filesystem does not support no-replace renames")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/workspace-tests");
        assert!(self.0.starts_with(&base) && self.0 != base);
        fs::remove_dir_all(&self.0).unwrap();
        let home = self.0.with_extension("home");
        if home.exists() {
            fs::remove_dir_all(home).unwrap();
        }
    }
}
