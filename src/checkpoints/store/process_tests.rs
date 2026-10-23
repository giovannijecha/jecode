use super::*;
use std::process::{Child, Command, Stdio};

#[test]
fn cross_process_reader_delays_expiry_until_release() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("data"), b"before").unwrap();
    let store = fixture.store();
    let cancel = Cancellation::default();
    let captured = store
        .capture("write", "test", &["data".into()], &cancel)
        .unwrap();
    let reference = captured.reference.unwrap();
    let ready = fixture.0.path().join("reader-ready");
    let mut child = spawn_child("reader", &fixture, &reference, &ready);
    wait_ready(&mut child, &ready);
    assert_eq!(store.cleanup(captured.created + TTL_MILLIS).unwrap(), 0);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(store.cleanup(captured.created + TTL_MILLIS).unwrap(), 1);
}

#[test]
fn crashed_process_pending_file_is_reaped_after_its_lock_releases() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let reference = "rc-0000000000000001-00000001-0000000000000001";
    let ready = fixture.0.path().join("pending-ready");
    let mut child = spawn_child("pending", &fixture, reference, &ready);
    wait_ready(&mut child, &ready);
    assert_eq!(store.cleanup(TTL_MILLIS + 1).unwrap(), 0);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(store.cleanup(TTL_MILLIS + 1).unwrap(), 1);
}

#[test]
fn concurrent_process_writers_publish_distinct_immutable_packs() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace().join("data"), b"before").unwrap();
    let store = fixture.store();
    let ready = fixture.0.path().join("writer-ready");
    let mut child = spawn_child("writer", &fixture, "unused", &ready);
    let parent_ref = store
        .capture(
            "write",
            "parent",
            &["data".into()],
            &Cancellation::default(),
        )
        .unwrap()
        .reference
        .unwrap();
    wait_ready(&mut child, &ready);
    assert!(child.wait().unwrap().success());
    let child_ref = fs::read_to_string(&ready).unwrap();
    assert_ne!(parent_ref, child_ref);
    for reference in [&parent_ref, &child_ref] {
        let loaded = store.load(reference, &Cancellation::default()).unwrap();
        loaded
            .verify(&loaded.snapshot.entries["data"], &Cancellation::default())
            .unwrap();
    }
}

fn spawn_child(mode: &str, fixture: &Fixture, reference: &str, ready: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("recovery_child_process")
        .arg("--nocapture")
        .env("JECODE_RCP_CHILD_MODE", mode)
        .env("JECODE_RCP_CHILD_HOME", fixture.0.path().join("home"))
        .env("JECODE_RCP_CHILD_ROOT", fixture.workspace())
        .env("JECODE_RCP_CHILD_REF", reference)
        .env("JECODE_RCP_CHILD_READY", ready)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_ready(child: &mut Child, ready: &Path) {
    let started = std::time::Instant::now();
    while !ready.exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "recovery child exited before ready"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "recovery child did not become ready"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn recovery_child_process() {
    let Ok(mode) = std::env::var("JECODE_RCP_CHILD_MODE") else {
        return;
    };
    let home = PathBuf::from(std::env::var_os("JECODE_RCP_CHILD_HOME").unwrap());
    let root = PathBuf::from(std::env::var_os("JECODE_RCP_CHILD_ROOT").unwrap());
    let reference = std::env::var("JECODE_RCP_CHILD_REF").unwrap();
    let ready = PathBuf::from(std::env::var_os("JECODE_RCP_CHILD_READY").unwrap());
    let store = Store::open(&home, &root).unwrap();
    let _held = match mode.as_str() {
        "writer" => {
            let captured = store
                .capture("write", "child", &["data".into()], &Cancellation::default())
                .unwrap();
            fs::write(&ready, captured.reference.unwrap()).unwrap();
            return;
        }
        "reader" => {
            let loaded = store.load(&reference, &Cancellation::default()).unwrap();
            fs::write(&ready, b"ready").unwrap();
            Some(loaded)
        }
        "pending" => {
            let path = store.directory.join(format!(".{reference}.pending"));
            let mut file = files::options()
                .write(true)
                .create_new(true)
                .open(path)
                .unwrap();
            file.lock().unwrap();
            file.write_all(b"interrupted").unwrap();
            fs::write(&ready, b"ready").unwrap();
            // Keep the handle alive through the simulated crash.
            std::thread::sleep(std::time::Duration::from_secs(10));
            drop(file);
            None
        }
        _ => panic!("unknown recovery child mode"),
    };
    std::thread::sleep(std::time::Duration::from_secs(10));
}
