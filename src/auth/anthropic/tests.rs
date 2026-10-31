use super::*;

#[test]
fn credentials_persist_and_sign_out_removes_local_tokens() {
    let directory = crate::test_support::Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let account =
        Store::fixture_account(directory.path().to_path_buf(), "test-access-token").unwrap();
    let session = store.session(&account.id).unwrap();
    assert_eq!(session.id(), "fixture-account");
    assert_eq!(session.identity(), account.id);
    assert_eq!(
        session.access_token(&Cancellation::default()).unwrap(),
        "test-access-token"
    );
    store.activate(session.id()).unwrap();
    assert!(store.active().unwrap().is_some());
    assert_eq!(session.sign_out(&Cancellation::default()).unwrap(), None);
    assert!(
        !store
            .session(session.id())
            .unwrap()
            .account()
            .unwrap()
            .enabled
    );
    assert!(store.active().unwrap().is_none());
    assert!(session.access_token(&Cancellation::default()).is_err());
}

#[test]
fn account_lock_serializes_access() {
    let directory = crate::test_support::Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let first = store.files().lock(&Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    let waiting = cancel.clone();
    let second = store.clone();
    let worker = std::thread::spawn(move || second.files().lock(&waiting).map(|_| ()));
    std::thread::sleep(std::time::Duration::from_millis(100));
    cancel.cancel();
    assert_eq!(worker.join().unwrap(), Err("Operation cancelled".into()));
    drop(first);
    assert!(store.files().lock(&Cancellation::default()).is_ok());
}

#[test]
fn stored_account_is_redacted_even_when_signed_out() {
    let directory = crate::test_support::Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    Store::fixture_account(directory.path().to_path_buf(), "test-access-token").unwrap();
    let redactor = store.redactor().unwrap();
    assert!(
        !redactor
            .text("test-access-token")
            .contains("test-access-token")
    );
}

#[test]
fn malformed_account_is_counted_without_hiding_valid_accounts() {
    let directory = crate::test_support::Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    Store::fixture_account(directory.path().to_path_buf(), "test-access-token").unwrap();
    std::fs::write(
        directory
            .path()
            .join("accounts/anthropic/account-invalid.json"),
        "not-json",
    )
    .unwrap();
    let (_, invalid) = store.redactor_report().unwrap();
    assert_eq!(invalid, 1);
    assert_eq!(store.accounts().unwrap().len(), 1);
}
