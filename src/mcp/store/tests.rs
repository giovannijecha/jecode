use super::*;
use crate::mcp::types::AuthMethod;

fn fixture() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "jecode-mcp-store-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}
fn draft(name: &str) -> Connection {
    Connection::draft(name, "https://example.test/mcp")
}

#[test]
fn round_trip_preserves_unknown_fields_and_independent_configuration() {
    let home = fixture();
    let store = Store::new(home.clone());
    fs::write(
        store.path(),
        r#"{"future":{"enabled":true},"connections":[]}"#,
    )
    .unwrap();
    let mut connection = draft("Example");
    connection.method = AuthMethod::Token;
    connection.token = "synthetic-token".into();
    let saved = store.save(&connection).unwrap();
    assert!(!saved.id.is_empty());
    assert_eq!(store.list().unwrap()[0].token, "synthetic-token");
    let mut document = json::parse(&fs::read_to_string(store.path()).unwrap()).unwrap();
    if let Value::Object(root) = &mut document
        && let Some(Value::Array(connections)) = root.get_mut("connections")
        && let Value::Object(entry) = &mut connections[0]
    {
        entry.insert("future_connection_field".into(), Value::Bool(true));
    }
    fs::write(store.path(), document.encode()).unwrap();
    store.set_enabled(&saved.id, false).unwrap();
    assert!(!store.list().unwrap()[0].enabled);
    let text = fs::read_to_string(store.path()).unwrap();
    assert!(text.contains("\"future\""));
    assert!(text.contains("\"future_connection_field\""));
    assert!(!home.join("config.json").exists());
    store.remove(&saved.id).unwrap();
    assert!(store.list().unwrap().is_empty());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn duplicate_names_and_invalid_headers_leave_existing_file_unchanged() {
    let home = fixture();
    let store = Store::new(home.clone());
    store.save(&draft("One")).unwrap();
    let before = fs::read(store.path()).unwrap();
    assert!(store.save(&draft("one")).is_err());
    let mut bad = draft("Two");
    bad.headers.push(("Authorization".into(), "secret".into()));
    assert!(store.save(&bad).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), before);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn oauth_session_without_expiry_round_trips() {
    let home = fixture();
    let store = Store::new(home.clone());
    let mut connection = draft("OAuth");
    connection.method = AuthMethod::Browser;
    connection.oauth = Some(crate::mcp::types::OAuthTokens {
        endpoint: connection.endpoint.clone(),
        access_token: "synthetic-access".into(),
        refresh_token: None,
        expires_at: None,
        token_endpoint: "https://issuer.test/token".into(),
        client_id: "synthetic-client".into(),
        issuer: "https://issuer.test".into(),
        scope: None,
        resource: Some(connection.endpoint.clone()),
        github: false,
    });
    let saved = store.save(&connection).unwrap();
    assert!(
        fs::read_to_string(store.path())
            .unwrap()
            .contains("\"expires_at\": null")
    );
    assert!(store.list().unwrap() == vec![saved.clone()]);
    assert_eq!(store.save(&draft("Other")).unwrap().name, "Other");
    assert!(store.fresh(&saved.id, &Cancellation::default()).unwrap() == saved);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn fresh_waits_for_the_store_lock_cancellably() {
    let home = fixture();
    let store = Store::new(home.clone());
    let guard = store.lock().unwrap();
    let cancel = Cancellation::default();
    let trigger = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        trigger.cancel();
    });
    let error = store.fresh("mcp-any", &cancel).map(drop).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    canceller.join().unwrap();
    drop(guard);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn hop_by_hop_and_proxy_headers_are_reserved() {
    for name in [
        "Transfer-Encoding",
        "TE",
        "Upgrade",
        "Expect",
        "Proxy-Authorization",
        "Connection",
        "Content-Length",
    ] {
        let mut connection = draft("Headers");
        connection.headers.push((name.into(), "value".into()));
        assert!(config::validate(&connection).is_err(), "{name}");
    }
}

#[test]
fn corrupt_file_and_disabled_connection_cannot_be_silently_replaced() {
    let home = fixture();
    let store = Store::new(home.clone());
    fs::write(store.path(), "not json").unwrap();
    assert!(store.save(&draft("One")).is_err());
    assert_eq!(fs::read_to_string(store.path()).unwrap(), "not json");
    fs::remove_file(store.path()).unwrap();
    let saved = store.save(&draft("One")).unwrap();
    store.set_enabled(&saved.id, false).unwrap();
    assert!(store.fresh(&saved.id, &Cancellation::default()).is_err());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn edits_keep_rotated_oauth_tokens_and_new_sign_ins_replace_them() {
    let home = fixture();
    let store = Store::new(home.clone());
    let tokens = |access: &str, refresh: &str| crate::mcp::types::OAuthTokens {
        endpoint: "https://example.test/mcp".into(),
        access_token: access.into(),
        refresh_token: Some(refresh.into()),
        expires_at: Some(u64::MAX / 2),
        token_endpoint: "https://auth.example.test/token".into(),
        client_id: "fixture-client".into(),
        issuer: "https://auth.example.test".into(),
        scope: None,
        resource: Some("https://example.test/mcp".into()),
        github: false,
    };
    let mut connection = draft("OAuth");
    connection.method = AuthMethod::Browser;
    connection.oauth = Some(tokens("isolated-old-access", "isolated-old-refresh"));
    let loaded = store.save(&connection).unwrap();
    // A refresh elsewhere rotates both tokens after the edit was loaded.
    let mut rotated = loaded.clone();
    rotated.oauth = Some(tokens("isolated-new-access", "isolated-new-refresh"));
    store.save(&rotated).unwrap();
    let mut renamed = loaded.clone();
    renamed.name = "OAuth renamed".into();
    let saved = store.save_edit(&renamed).unwrap();
    assert!(saved.oauth == rotated.oauth);
    let listed = store.list().unwrap();
    assert_eq!(listed[0].name, "OAuth renamed");
    assert!(listed[0].oauth == rotated.oauth);
    // A new sign-in replaces the stored tokens.
    let mut signed = saved.clone();
    signed.oauth = Some(tokens("isolated-signed-access", "isolated-signed-refresh"));
    store.save(&signed).unwrap();
    assert!(store.list().unwrap()[0].oauth == signed.oauth);
    fs::remove_dir_all(home).unwrap();
}
