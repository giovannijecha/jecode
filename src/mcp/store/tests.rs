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
