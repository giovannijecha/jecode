#[cfg(target_os = "linux")]
#[path = "support/workspace.rs"]
mod support;
#[cfg(target_os = "linux")]
mod linux {
    use super::support;
    use jecode::{
        state::Store,
        workspace::{Budget, RecoveryStore, Workspace},
    };
    use std::{
        fs,
        os::unix::fs::MetadataExt,
        path::{Path, PathBuf},
        sync::atomic::{AtomicBool, AtomicU64, Ordering},
        time::{Duration, Instant},
    };

    struct IsolatedHome(PathBuf);
    impl Drop for IsolatedHome {
        fn drop(&mut self) {
            if self.0.parent() == Some(Path::new("/dev/shm"))
                && self
                    .0
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("jecode-recovery-test-")
            {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }

    #[test]
    fn private_state_on_another_filesystem_uses_copy_not_cross_device_rename() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let files = support::Fixture::new();
        let base = Path::new("/dev/shm");
        let Ok(meta) = fs::metadata(base) else {
            return;
        };
        if meta.dev() == fs::metadata(&files.0).unwrap().dev() {
            return;
        }
        let home = base.join(format!(
            "jecode-recovery-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        if fs::create_dir(&home).is_err() {
            return;
        }
        let home = IsolatedHome(home);
        let recoveries = RecoveryStore::in_store(&Store::in_home(&home.0).unwrap()).unwrap();
        files.write("index.html", "before\n");
        let ws = Workspace::open(&files.0).unwrap();
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        let change = ws
            .prepare_edit("index.html", "before", "after", &budget)
            .unwrap();
        let id = match ws.apply(change, &budget, &recoveries, Some("cross-fs"), "edit") {
            Ok(applied) => applied.recovery.unwrap(),
            Err(error) if files.unsupported_host_filesystem(&error.to_string()) => {
                assert_eq!(fs::read(files.0.join("index.html")).unwrap(), b"before\n");
                return;
            }
            Err(error) => panic!("{error}"),
        };
        assert_eq!(
            fs::read(recoveries.root().join(format!("{id}.before"))).unwrap(),
            b"before\n"
        );
        assert_eq!(fs::read(files.0.join("index.html")).unwrap(), b"after\n");
        assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
    }
}
