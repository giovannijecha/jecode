use super::*;
use crate::test_support::Directory;

#[test]
fn cancelling_remote_revocation_still_removes_local_tokens_and_retains_registration() {
    use crate::test_support::server::{Server, accept};
    use std::io::{BufRead, BufReader, Write};
    let _fixture_lock = FIXTURE_SERIAL.lock().unwrap();
    let session = Session::fixture("revoked-access-token");
    session.store.activate(session.client_id()).unwrap();
    let (ready, received) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let server = Server::start(move |listener, stop| {
        for index in 0..2 {
            let mut stream = accept(&listener, &stop).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            if index == 0 {
                let discovery = crate::json::Value::object([
                    ("issuer", crate::json::Value::string(wire::AUTHORITY)),
                    (
                        "authorization_endpoint",
                        crate::json::Value::string(wire::AUTHORIZE),
                    ),
                    ("token_endpoint", crate::json::Value::string(wire::TOKEN)),
                    (
                        "revocation_endpoint",
                        crate::json::Value::string(format!(
                            "{}/api/accounts/oauth/revoke",
                            wire::AUTHORITY
                        )),
                    ),
                ])
                .encode();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{discovery}",
                    discovery.len()
                )
                .unwrap();
            } else {
                ready.send(()).unwrap();
                released
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
            }
        }
    });
    wire::set_fixture(Some(server.endpoint.clone()));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            wire::set_fixture(None);
        }
    }
    let _reset = Reset;
    let cancel = Cancellation::default();
    let worker_session = session.clone();
    let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || worker_session.sign_out(&worker_cancel));
    received
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    cancel.cancel();
    release.send(()).unwrap();
    assert!(worker.join().unwrap().unwrap().is_some());
    server.finish();
    assert!(session.store.active().unwrap().is_none());
    assert_eq!(session.store.accounts().unwrap().len(), 1);
    assert!(!session.account().unwrap().enabled);
    assert!(session.access_token(&Cancellation::default()).is_err());
}

#[test]
fn inactive_registration_tokens_are_masked_before_a_provider_is_selected() {
    let directory = Directory::new();
    Store::fixture_account(directory.path().to_path_buf(), "first-account-access").unwrap();
    let store = Store::new(directory.path().to_path_buf());
    let files = store.files();
    let mut record = files
        .record(
            "fixture-client",
            "fixture-subject",
            &Cancellation::default(),
        )
        .unwrap();
    record.client_id = "inactive-client".into();
    record.access_token = "inactive-access-token".into();
    record.refresh_token = "inactive-refresh-token".into();
    record.id_token = "inactive-id-token".into();
    files
        .save_record(&record, &Cancellation::default())
        .unwrap();
    assert_eq!(
        store.active().unwrap().unwrap().client_id(),
        "fixture-client"
    );
    assert_eq!(
        store.redactor().unwrap().text(
            "first-account-access inactive-access-token inactive-refresh-token inactive-id-token"
        ),
        "[redacted] [redacted] [redacted] [redacted]"
    );
}
static FIXTURE_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn account_records_keep_client_and_subject_separate_and_active_is_explicit() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let files = store.files();
    let host = files.host_id(&Cancellation::default()).unwrap();
    let make = |client: &str, subject: &str| Record {
        client_id: client.into(),
        subject: subject.into(),
        email: "same@example.test".into(),
        label: "Same".into(),
        host_id: host.clone(),
        id_token: "id-secret".into(),
        access_token: "access-secret".into(),
        refresh_token: "refresh-secret".into(),
        scopes: vec!["chatgpt.tokens.use.direct".into()],
        expires_at: u64::MAX,
        earliest_refresh_at: 0,
    };
    files
        .save_record(&make("client-a", "subject-1"), &Cancellation::default())
        .unwrap();
    files
        .save_record(&make("client-b", "subject-1"), &Cancellation::default())
        .unwrap();
    files
        .save_record(&make("client-c", "subject-2"), &Cancellation::default())
        .unwrap();
    let accounts = store.accounts().unwrap();
    assert_eq!(accounts.len(), 3);
    assert_eq!(
        accounts
            .iter()
            .map(|account| &account.label)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
    assert!(store.active().unwrap().is_none());
    store.activate("client-b").unwrap();
    assert_eq!(
        store.active().unwrap().unwrap().identity(),
        "client-b:subject-1"
    );
    let session = store.session("client-a").unwrap();
    assert_eq!(
        session.access_token(&Cancellation::default()).unwrap(),
        "access-secret"
    );
    assert_eq!(
        session
            .redactor()
            .text("access-secret refresh-secret id-secret"),
        "[redacted] [redacted] [redacted]"
    );
}

#[test]
fn cancelled_lock_wait_does_not_use_stale_tokens() {
    let directory = Directory::new();
    let files = Files::new(directory.path().to_path_buf());
    let _held = files.lock(&Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(files.lock(&cancel).unwrap_err().contains("cancelled"));
}

#[test]
fn native_acl_boundary_accepts_unicode_account_paths() {
    let directory = Directory::new();
    let store = Store::new(directory.path().join("Giovanni-é-用户"));
    let account = Store::fixture_account(store.home.clone(), "unicode-secret").unwrap();
    assert!(account.enabled);
    assert_eq!(
        store
            .active()
            .unwrap()
            .unwrap()
            .access_token(&Cancellation::default())
            .unwrap(),
        "unicode-secret"
    );
}

#[test]
fn url_encoding_and_duplicate_callback_fields_are_strict() {
    assert_eq!(
        wire::form(&[("scope", "openid email"), ("state", "a+b")]),
        "scope=openid%20email&state=a%2Bb"
    );
    assert_eq!(
        wire::query("state=a%2Bb&scope=openid+email").unwrap()["state"],
        "a+b"
    );
    assert!(wire::query("state=a&state=b").is_err());
    assert!(wire::query("state=%XZ").is_err());
}

#[test]
fn concurrent_refresh_reloads_rotated_tokens_under_one_lock() {
    use crate::test_support::server::{Server, accept};
    use std::io::{Read, Write};
    let _fixture_lock = FIXTURE_SERIAL.lock().unwrap();
    let session = Session::fixture("old-access");
    let files = session.store.files();
    let mut record = files
        .record(
            session.client_id(),
            "fixture-subject",
            &Cancellation::default(),
        )
        .unwrap();
    record.expires_at = 0;
    record.earliest_refresh_at = 0;
    files
        .save_record(&record, &Cancellation::default())
        .unwrap();
    let server = Server::start(|listener, stop| {
        let mut stream = accept(&listener, &stop).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 2048];
        loop {
            let size = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..size]);
            if request.windows(4).any(|part| part == b"\r\n\r\n") {
                break;
            }
        }
        let header_end = request
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let length: usize = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|n| n.trim().parse().ok())
            })
            .unwrap();
        while request.len() < header_end + length {
            let size = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..size]);
        }
        let body = String::from_utf8_lossy(&request[header_end..header_end + length]);
        assert!(body.contains("refresh_token=fixture-refresh"));
        let response = r#"{"access_token":"new-access","refresh_token":"new-refresh","token_type":"Bearer","scope":"chatgpt.tokens.use.direct","expires_in":3600,"earliest_refresh_at":0}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        1usize
    });
    wire::set_fixture(Some(server.endpoint.clone()));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            wire::set_fixture(None);
        }
    }
    let _reset = Reset;
    let a = session.clone();
    let b = session.clone();
    let first = std::thread::spawn(move || a.access_token(&Cancellation::default()));
    let second = std::thread::spawn(move || b.access_token(&Cancellation::default()));
    assert_eq!(first.join().unwrap().unwrap(), "new-access");
    assert_eq!(second.join().unwrap().unwrap(), "new-access");
    assert_eq!(server.finish(), 1);
    let saved = files
        .record(
            session.client_id(),
            "fixture-subject",
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(saved.refresh_token, "new-refresh");
    assert_eq!(
        session
            .redactor()
            .text("old-access new-access fixture-refresh new-refresh"),
        "[redacted] [redacted] [redacted] [redacted]"
    );
}

#[test]
fn refresh_errors_clear_only_confirmed_unusable_tokens() {
    use crate::test_support::server::{Server, accept};
    use std::io::{Read, Write};
    let _fixture_lock = FIXTURE_SERIAL.lock().unwrap();
    for (code, status, clears) in [
        ("refresh_token_reused", 400, true),
        ("invalid_client", 401, false),
        ("service_unavailable", 503, false),
    ] {
        let session = Session::fixture("old-access");
        let files = session.store.files();
        let mut record = files
            .record(
                session.client_id(),
                "fixture-subject",
                &Cancellation::default(),
            )
            .unwrap();
        record.expires_at = 0;
        files
            .save_record(&record, &Cancellation::default())
            .unwrap();
        let response = format!("{{\"error\":\"{code}\"}}");
        let server = Server::start(move |listener, stop| {
            let mut stream = accept(&listener, &stop).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            write!(stream, "HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        });
        wire::set_fixture(Some(server.endpoint.clone()));
        let result = session.access_token(&Cancellation::default());
        wire::set_fixture(None);
        server.finish();
        assert!(result.is_err(), "{code}");
        let stored = files
            .record(
                session.client_id(),
                "fixture-subject",
                &Cancellation::default(),
            )
            .unwrap();
        assert_eq!(!stored.enabled(), clears, "{code}");
    }
    assert!(!unusable_refresh(
        "OpenAI identity request failed: invalid_client"
    ));
    assert!(!unusable_refresh(
        "OpenAI identity request failed: invalid_grant_extra"
    ));
}

#[test]
fn access_tokens_come_from_memory_until_the_record_changes() {
    let cancel = Cancellation::default();
    let session = Session::fixture("first-access");
    assert_eq!(session.access_token(&cancel).unwrap(), "first-access");
    assert_eq!(session.cached_token().as_deref(), Some("first-access"));
    // Another process signing out rewrites the record and invalidates the copy.
    let files = session.store.files();
    let mut record = files
        .record(session.client_id(), "fixture-subject", &cancel)
        .unwrap();
    record.access_token.clear();
    files.save_record(&record, &cancel).unwrap();
    assert_eq!(session.cached_token(), None);
    assert!(session.access_token(&cancel).is_err());
    record.access_token = "second-access".into();
    files.save_record(&record, &cancel).unwrap();
    assert_eq!(session.access_token(&cancel).unwrap(), "second-access");
    // Tokens near expiry always take the refreshing disk path.
    record.expires_at = storage::now().unwrap() + 30;
    record.earliest_refresh_at = u64::MAX;
    files.save_record(&record, &cancel).unwrap();
    assert_eq!(session.access_token(&cancel).unwrap(), "second-access");
    assert_eq!(session.cached_token(), None);
}

#[test]
fn cancelled_access_does_not_return_a_cached_token() {
    let session = Session::fixture("fixture-access");
    let cancel = Cancellation::default();
    assert_eq!(session.access_token(&cancel).unwrap(), "fixture-access");
    cancel.cancel();
    assert_eq!(
        session.access_token(&cancel).unwrap_err(),
        "Operation cancelled"
    );
}

#[test]
fn token_cache_stamp_rejects_a_directory() {
    let directory = crate::test_support::Directory::new();
    assert!(stamp(directory.path()).is_none());
}

#[cfg(unix)]
#[test]
fn cached_access_reprotects_a_record_with_changed_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let session = Session::fixture("fixture-access");
    let cancel = Cancellation::default();
    assert_eq!(session.access_token(&cancel).unwrap(), "fixture-access");
    let path = session
        .store
        .files()
        .record_path(session.client_id(), "fixture-subject", &cancel)
        .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(session.access_token(&cancel).unwrap(), "fixture-access");
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
