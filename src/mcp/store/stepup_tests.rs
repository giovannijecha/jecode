use super::*;
use crate::mcp::types::{AuthMethod, ClientAuthMethod, OAuthTokens};
use crate::test_support::Directory;

fn connection() -> Connection {
    let endpoint = "http://127.0.0.1:9/mcp";
    let mut connection = Connection::draft("Scoped", endpoint);
    connection.method = AuthMethod::Browser;
    connection.oauth = Some(OAuthTokens {
        endpoint: endpoint.into(),
        access_token: "fixture-access-1".into(),
        refresh_token: Some("fixture-refresh-1".into()),
        expires_at: Some(4_000_000_000),
        token_endpoint: "http://127.0.0.1:9/token".into(),
        client_id: "fixture-client".into(),
        client_auth_method: ClientAuthMethod::None,
        client_secret: None,
        client_secret_expires_at: None,
        issuer: "http://127.0.0.1:9".into(),
        scope: Some("read".into()),
        resource: Some(endpoint.into()),
        github: false,
    });
    connection
}

#[test]
fn challenged_scope_is_merged_and_survives_a_stale_form_edit() {
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf());
    let saved = store.save(&connection()).unwrap();
    let edit_opened_before_challenge = saved.clone();
    store.require_scope(&saved, "deploy read").unwrap();
    store.require_scope(&saved, "deploy admin").unwrap();
    assert_eq!(
        store.list().unwrap()[0].pending_scope.as_deref(),
        Some("deploy read admin")
    );
    store.save_edit(&edit_opened_before_challenge).unwrap();
    assert_eq!(
        store.list().unwrap()[0].pending_scope.as_deref(),
        Some("deploy read admin")
    );
}

#[test]
fn old_challenge_cannot_change_a_rotated_or_reconnected_oauth_session() {
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf());
    let old = store.save(&connection()).unwrap();
    let mut renewed = old.clone();
    renewed.oauth.as_mut().unwrap().access_token = "fixture-access-2".into();
    store.save(&renewed).unwrap();
    assert!(store.require_scope(&old, "admin").is_err());
    assert_eq!(store.list().unwrap()[0].pending_scope, None);
    let mut changed = renewed.clone();
    changed.endpoint = "http://127.0.0.1:9/other".into();
    changed.oauth = None;
    store.save(&changed).unwrap();
    assert!(store.require_scope(&renewed, "admin").is_err());
    assert_eq!(store.list().unwrap()[0].pending_scope, None);
}
