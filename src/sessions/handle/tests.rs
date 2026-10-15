use super::*;
use crate::{effort::Effort, json::Value, test_support::Directory};
use std::sync::mpsc;

fn fixture() -> (Directory, Directory, Handle) {
    let home = Directory::new();
    let directory = Directory::new();
    let store = Store::new(home.path().to_path_buf(), directory.path()).unwrap();
    let mut document = Document::new(
        directory.path().to_path_buf(),
        "fixture/model".into(),
        Effort::High,
    )
    .unwrap();
    for (role, content) in [("system", "fixture system"), ("user", "saved request")] {
        document.messages.push(Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(content)),
        ]));
    }
    let handle = Handle::new(store, document, Redactor::new("fixture-key".into()));
    (home, directory, handle)
}

#[test]
fn repeated_capture_keeps_staged_input_and_skips_unchanged_redaction() {
    let (_home, _directory, handle) = fixture();
    let staged = crate::attachments::Attachment {
        id: "att-123-1-0".into(),
        name: "example.txt".into(),
        media: "text/plain".into(),
        size: 3,
        width: None,
        height: None,
        pages: None,
    };
    handle.input(Input {
        staged: vec![staged.clone()],
        ..Input::default()
    });
    let input = Input {
        draft: super::super::Draft {
            text: "fixture-key draft".into(),
            cursor: "fixture-key draft".len(),
            ..Default::default()
        },
        ..Input::default()
    };
    handle.capture_input(input.clone());
    let revision = handle.0.live.lock().unwrap().revision;
    handle.capture_input(input);
    assert_eq!(handle.0.live.lock().unwrap().revision, revision);
    let saved = handle.snapshot().input;
    assert_eq!(saved.staged, [staged]);
    assert!(!saved.draft.text.contains("fixture-key"));
}

#[test]
fn composer_capture_can_proceed_while_a_save_waits_for_disk_access() {
    let (_home, _directory, handle) = fixture();
    let io = handle.0.io.lock().unwrap();
    let (started, running) = mpsc::channel();
    let writer = handle.clone();
    let save = std::thread::spawn(move || {
        started.send(()).unwrap();
        writer.flush().unwrap();
    });
    running.recv().unwrap();
    let (done, captured) = mpsc::channel();
    let editor = handle.clone();
    let capture = std::thread::spawn(move || {
        editor.capture_input(Input {
            draft: super::super::Draft {
                text: "last edit".into(),
                cursor: 9,
                ..Default::default()
            },
            ..Input::default()
        });
        done.send(()).unwrap();
    });
    let completed_while_io_is_busy = captured.recv_timeout(Duration::from_secs(2)).is_ok();
    drop(io);
    capture.join().unwrap();
    save.join().unwrap();
    assert!(completed_while_io_is_busy);
    handle.flush().unwrap();
    let recovered = handle.store().fixture_load(&handle.id()).unwrap();
    assert_eq!(recovered.input.draft.text, "last edit");
}

#[test]
fn background_save_is_polled_without_waiting_and_exit_flushes_the_last_edit() {
    let (_home, _directory, handle) = fixture();
    handle.capture_input(Input {
        draft: super::super::Draft {
            text: "early edit".into(),
            cursor: 10,
            ..Default::default()
        },
        ..Input::default()
    });
    handle.0.live.lock().unwrap().dirty = Some(Instant::now() - SAVE_INTERVAL);
    let io = handle.0.io.lock().unwrap();
    assert!(!handle.flush_due_background().unwrap());
    handle.capture_input(Input {
        draft: super::super::Draft {
            text: "final edit".into(),
            cursor: 10,
            ..Default::default()
        },
        ..Input::default()
    });
    drop(io);
    handle.flush().unwrap();
    let recovered = handle.store().fixture_load(&handle.id()).unwrap();
    assert_eq!(recovered.input.draft.text, "final edit");
}

#[test]
fn failed_background_save_reports_error_and_keeps_the_latest_edit_for_retry() {
    let (_home, _directory, handle) = fixture();
    handle.flush().unwrap();
    let path = handle
        .store()
        .bucket()
        .join(format!("{}.jsonl", handle.id()));
    let original = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();
    handle.capture_input(Input {
        draft: super::super::Draft {
            text: "retry me".into(),
            cursor: 8,
            ..Default::default()
        },
        ..Input::default()
    });
    handle.0.live.lock().unwrap().dirty = Some(Instant::now() - SAVE_INTERVAL);
    assert!(!handle.flush_due_background().unwrap());
    let deadline = Instant::now() + Duration::from_secs(2);
    let error = loop {
        match handle.flush_due_background() {
            Err(error) => break error,
            Ok(_) => {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
    };
    assert!(error.contains("read-only"));
    assert!(handle.0.live.lock().unwrap().dirty.is_some());
    std::fs::set_permissions(&path, original).unwrap();
    handle.flush().unwrap();
    assert_eq!(
        handle
            .store()
            .fixture_load(&handle.id())
            .unwrap()
            .input
            .draft
            .text,
        "retry me"
    );
}
