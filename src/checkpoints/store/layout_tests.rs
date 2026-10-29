use super::*;

fn archive_path(home: &Path, workspace: &Path) -> PathBuf {
    let root = fs::canonicalize(workspace).unwrap();
    let identity = super::super::identity::identity_bytes(&root);
    let key = super::super::hash::digest(&identity);
    let leaf = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    let friendly: String = leaf
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(32)
        .collect();
    let friendly = if friendly.is_empty() {
        "workspace"
    } else {
        &friendly
    };
    home.join("checkpoints")
        .join(format!("{friendly}-{:016x}{:016x}", key.0[0], key.0[1]))
}

#[test]
fn fresh_store_keeps_packs_at_root_and_runtime_files_internal() {
    let fixture = Fixture::new();
    let home = fixture.0.path().join("home");
    let workspace = fixture.workspace();
    fs::write(workspace.join("data"), b"before").unwrap();
    let store = fixture.store();
    let archive = archive_path(&home, &workspace);
    assert_eq!(store.directory, fs::canonicalize(&archive).unwrap());
    let internal = archive.join(".internal");
    assert_eq!(
        fs::read(internal.join("workspace.id")).unwrap(),
        super::super::identity::identity_bytes(&store.root)
    );
    assert!(internal.join("identity.lock").is_file());

    let cancel = Cancellation::default();
    let lease = store
        .leases
        .acquire_cancellable(&["data"], &cancel)
        .unwrap();
    assert!(internal.join("leases").is_dir());
    drop(lease);
    let reference = store
        .capture("write", "test", &["data".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    assert!(archive.join(format!("{reference}.rcp")).is_file());
    for name in ["workspace.id", ".identity.lock", ".publish.lock", "leases"] {
        assert!(
            !archive.join(name).exists(),
            "unexpected archive root entry: {name}"
        );
    }
}

#[test]
fn opening_store_leaves_other_home_data_alone() {
    let fixture = Fixture::new();
    let home = fixture.0.path().join("home");
    fs::write(home.join("config.json"), b"personal config").unwrap();
    fs::create_dir(home.join("sessions")).unwrap();
    fs::write(home.join("sessions").join("kept"), b"session").unwrap();

    let _store = fixture.store();
    assert_eq!(
        fs::read(home.join("config.json")).unwrap(),
        b"personal config"
    );
    assert_eq!(
        fs::read(home.join("sessions").join("kept")).unwrap(),
        b"session"
    );
    for name in ["skills", "cache", "tmp"] {
        assert!(
            !home.join(name).exists(),
            "created unrelated home entry: {name}"
        );
    }
}

#[test]
fn restart_reuses_workspace_archive_without_sharing_another_workspace() {
    let fixture = Fixture::new();
    let home = fixture.0.path().join("home");
    fs::write(fixture.workspace().join("data"), b"before").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let reference = store
        .capture("write", "first-session", &["data".into()], &cancel)
        .unwrap()
        .reference
        .unwrap();
    let directory = store.directory.clone();
    drop(store);

    let reopened = Store::open(&home, &fixture.workspace()).unwrap();
    assert_eq!(reopened.directory, directory);
    assert!(reopened.load(&reference, &cancel).is_ok());

    let other_workspace = fixture.0.path().join("other-workspace");
    fs::create_dir(&other_workspace).unwrap();
    let other = Store::open(&home, &other_workspace).unwrap();
    assert_ne!(other.directory, directory);
    assert!(other.list(10, &cancel).unwrap().is_empty());
    assert!(other.load(&reference, &cancel).is_err());
}

#[test]
fn sweep_expires_recognized_pack_and_preserves_foreign_storage() {
    let fixture = Fixture::new();
    let home = fixture.0.path().join("home");
    fs::write(fixture.workspace().join("data"), b"before").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let captured = store
        .capture("write", "test", &["data".into()], &cancel)
        .unwrap();
    let reference = captured.reference.unwrap();
    let pack = store.directory.join(format!("{reference}.rcp"));
    let unknown = store.directory.join("keep.txt");
    fs::write(&unknown, b"unknown").unwrap();
    let legacy = home
        .join("checkpoints")
        .join("0123456789abcdef0123456789abcdef");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("history"), b"old format").unwrap();
    let foreign = home
        .join("checkpoints")
        .join("foreign-00000000000000000000000000000000");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join(format!("{reference}.rcp")), b"foreign pack").unwrap();
    fs::write(home.join("config.json"), b"personal config").unwrap();
    drop(store);

    assert_eq!(
        Store::sweep_home(&home, captured.created + TTL_MILLIS, &cancel).unwrap(),
        1
    );
    assert!(!pack.exists());
    assert_eq!(fs::read(unknown).unwrap(), b"unknown");
    assert_eq!(fs::read(legacy.join("history")).unwrap(), b"old format");
    assert_eq!(
        fs::read(foreign.join(format!("{reference}.rcp"))).unwrap(),
        b"foreign pack"
    );
    assert_eq!(
        fs::read(home.join("config.json")).unwrap(),
        b"personal config"
    );
}

#[cfg(unix)]
#[test]
fn unreadable_foreign_archive_does_not_stop_other_expiry() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let home = fixture.0.path().join("home");
    let store = fixture.store();
    let captured = store
        .capture(
            "write",
            "test",
            &["missing".into()],
            &Cancellation::default(),
        )
        .unwrap();
    let foreign = home.join("checkpoints/foreign-00000000000000000000000000000000/.internal");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("workspace.id"), b"unreadable").unwrap();
    fs::set_permissions(&foreign, fs::Permissions::from_mode(0o0)).unwrap();
    let restricted = fs::symlink_metadata(foreign.join("workspace.id")).is_err();
    let result = Store::sweep_home(
        &home,
        captured.created + TTL_MILLIS,
        &Cancellation::default(),
    );
    fs::set_permissions(&foreign, fs::Permissions::from_mode(0o700)).unwrap();
    if restricted {
        assert_eq!(result.unwrap(), 1);
    }
}

#[cfg(unix)]
#[test]
fn inherited_capture_descriptor_does_not_block_expiry_after_publication() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let inherited = Arc::new(Mutex::new(None::<File>));
    let retained = Arc::clone(&inherited);
    crate::checkpoints::capture::BEFORE_RELEASE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |file| {
            *retained.lock().unwrap() = Some(file.try_clone().unwrap());
        }));
    });
    let captured = store
        .capture(
            "write",
            "test",
            &["missing".into()],
            &Cancellation::default(),
        )
        .unwrap();
    assert!(inherited.lock().unwrap().is_some());
    assert!(captured.reference.is_some());
    assert_eq!(store.cleanup(captured.created + TTL_MILLIS).unwrap(), 1);
    drop(inherited);
}

#[test]
fn occupied_unmarked_archive_is_not_claimed() {
    let fixture = Fixture::new();
    let home = fixture.0.path().join("home");
    let archive = archive_path(&home, &fixture.workspace());
    fs::create_dir_all(&archive).unwrap();
    fs::write(archive.join("unknown"), b"owned elsewhere").unwrap();

    assert!(Store::open(&home, &fixture.workspace()).is_err());
    assert_eq!(
        fs::read(archive.join("unknown")).unwrap(),
        b"owned elsewhere"
    );
    assert!(!archive.join(".internal").join("workspace.id").exists());
}

#[test]
fn interrupted_identity_initialization_is_completed() {
    let fixture = Fixture::new();
    let home = fixture.0.path().join("home");
    let archive = archive_path(&home, &fixture.workspace());
    let internal = archive.join(".internal");
    fs::create_dir_all(&internal).unwrap();
    // The current staging name and the name earlier versions left behind.
    let temporaries = [
        crate::disk::temporary_beside(&internal.join("workspace.id")),
        internal.join(".write-1-1-1.tmp"),
    ];
    for temporary in &temporaries {
        fs::write(temporary, b"incomplete identity").unwrap();
    }
    let store = fixture.store();
    assert!(temporaries.iter().all(|temporary| !temporary.exists()));
    assert_eq!(
        fs::read(internal.join("workspace.id")).unwrap(),
        super::super::identity::identity_bytes(&store.root)
    );
}

#[test]
fn concurrent_first_open_establishes_one_identity() {
    let fixture = Fixture::new();
    let home = fixture.0.path().join("home");
    let workspace = fixture.workspace();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let home = home.clone();
            let workspace = workspace.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                Store::open(&home, &workspace).map(|store| store.directory.clone())
            })
        })
        .collect();
    let directories: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap())
        .collect();
    let expected = fs::canonicalize(archive_path(&home, &workspace)).unwrap();
    assert!(directories.iter().all(|directory| directory == &expected));
    assert_eq!(fs::read_dir(home.join("checkpoints")).unwrap().count(), 1);
}

#[test]
fn mismatched_workspace_marker_is_not_replaced() {
    let fixture = Fixture::new();
    let home = fixture.0.path().join("home");
    let archive = archive_path(&home, &fixture.workspace());
    fs::create_dir_all(archive.join(".internal")).unwrap();
    let marker = archive.join(".internal").join("workspace.id");
    fs::write(&marker, b"another workspace").unwrap();

    assert!(Store::open(&home, &fixture.workspace()).is_err());
    assert_eq!(fs::read(marker).unwrap(), b"another workspace");
}

#[cfg(unix)]
fn link_directory(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).unwrap();
    true
}

#[cfg(windows)]
fn link_directory(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}

#[test]
fn linked_internal_directory_does_not_redirect_storage() {
    let fixture = Fixture::new();
    let home = fixture.0.path().join("home");
    let archive = archive_path(&home, &fixture.workspace());
    let outside = fixture.0.path().join("outside");
    fs::create_dir_all(&archive).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(
        outside.join("workspace.id"),
        super::super::identity::identity_bytes(&fs::canonicalize(fixture.workspace()).unwrap()),
    )
    .unwrap();
    if !link_directory(&outside, &archive.join(".internal")) {
        return;
    }

    assert!(Store::open(&home, &fixture.workspace()).is_err());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
    assert!(!outside.join("identity.lock").exists());
}

#[test]
fn linked_lease_directory_does_not_redirect_locks() {
    let fixture = Fixture::new();
    let home = fixture.0.path().join("home");
    let store = fixture.store();
    let archive = store.directory.clone();
    drop(store);
    let outside = fixture.0.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let leases = archive.join(".internal").join("leases");
    if leases.exists() {
        fs::remove_dir(&leases).unwrap();
    }
    if !link_directory(&outside, &leases) {
        return;
    }

    if let Ok(reopened) = Store::open(&home, &fixture.workspace()) {
        assert!(
            reopened
                .leases
                .acquire_cancellable(&["data"], &Cancellation::default())
                .is_err()
        );
    }
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}
