#![cfg(any(windows, target_os = "linux"))]
use super::*;

pub(crate) struct Fixture(pub PathBuf);
impl Fixture {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/state-tests");
        fs::create_dir_all(&base).unwrap();
        let path = base.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub(crate) fn store(&self) -> Option<Store> {
        match Store::in_home(&self.0) {
            Ok(store) => Some(store),
            Err(error)
                if cfg!(target_os = "linux")
                    && self.0.starts_with("/mnt/c/")
                    && error.kind() == io::ErrorKind::Unsupported =>
            {
                eprintln!("private state correctly refuses DrvFS without Unix permissions");
                None
            }
            Err(error) => panic!("fixture store failed: {error}"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/state-tests");
        assert!(self.0.starts_with(&base) && self.0 != base);
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn new_namespace_preserves_legacy_and_replaces_plain_json() {
    let fixture = Fixture::new();
    let legacy = fixture.0.join(".jecode");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("credentials.json"), "legacy-sentinel").unwrap();
    let Some(store) = fixture.store() else {
        return;
    };
    store
        .replace("credentials.json", "{\"version\":1}")
        .unwrap();
    store
        .replace("credentials.json", "{\"version\":1,\"account\":null}")
        .unwrap();
    assert_eq!(
        store.read("credentials.json", 64).unwrap().unwrap(),
        "{\"version\":1,\"account\":null}"
    );
    assert!(store.read("credentials.json", 8).is_err());
    assert_eq!(
        fs::read_to_string(legacy.join("credentials.json")).unwrap(),
        "legacy-sentinel"
    );
    assert_eq!(store.names().unwrap(), ["credentials.json"]);
    for name in [
        "../credentials.json",
        "x/y",
        "C:\\x",
        "x:secret",
        ".",
        "CON.json",
        "a.",
        "",
    ] {
        assert!(store.replace(name, "invalid").is_err());
    }
}

#[test]
fn lock_serializes_writers_and_drop_allows_next_owner() {
    let fixture = Fixture::new();
    let Some(store) = fixture.store() else {
        return;
    };
    let cancel = AtomicBool::new(false);
    let lock = store
        .lock("credentials.lock", &cancel, Instant::now())
        .unwrap();
    assert_eq!(
        store
            .lock("credentials.lock", &cancel, Instant::now())
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    drop(lock);
    let _next = store
        .lock("credentials.lock", &cancel, Instant::now())
        .unwrap();
    cancel.store(true, Ordering::Release);
    assert_eq!(
        store
            .lock("other.lock", &cancel, Instant::now())
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
}

#[cfg(target_os = "linux")]
#[test]
fn private_permissions_links_and_non_utf8_are_checked() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Fixture::new();
    let Some(store) = fixture.store() else {
        return;
    };
    store.replace("original.json", "{}").unwrap();
    let original = store.root.join("original.json");
    assert_eq!(
        fs::metadata(&original).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&store.root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    symlink(&original, store.root.join("link.json")).unwrap();
    assert!(store.read("link.json", 10).is_err());
    assert!(store.replace("link.json", "bad").is_err());
    fs::hard_link(&original, store.root.join("hard.json")).unwrap();
    assert!(store.read("original.json", 10).is_err());
    fs::remove_file(store.root.join("hard.json")).unwrap();
    fs::write(&original, [255]).unwrap();
    assert!(store.read("original.json", 10).is_err());
    fs::set_permissions(&original, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.read("original.json", 10).is_err());
}
