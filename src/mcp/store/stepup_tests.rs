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
    assert!(!store.require_scope(&old, "admin").unwrap());
    assert_eq!(store.list().unwrap()[0].pending_scope, None);
    let mut changed = renewed.clone();
    changed.endpoint = "http://127.0.0.1:9/other".into();
    changed.oauth = None;
    store.save(&changed).unwrap();
    assert!(!store.require_scope(&renewed, "admin").unwrap());
    assert_eq!(store.list().unwrap()[0].pending_scope, None);
}

#[test]
fn cached_sign_in_loads_the_latest_scope_and_rotated_tokens() {
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf());
    let cached = store.save(&connection()).unwrap();
    let mut refreshed = cached.clone();
    refreshed.oauth.as_mut().unwrap().access_token = "fixture-refreshed".into();
    store.save(&refreshed).unwrap();
    store.require_scope(&refreshed, "deploy").unwrap();

    let (prepared, guard) = store.prepare_sign_in(&cached, Some(&cached)).unwrap();
    assert_eq!(prepared.pending_scope.as_deref(), Some("deploy"));
    assert!(prepared.oauth == refreshed.oauth);
    assert!(guard.is_some());
}

#[test]
fn challenge_during_sign_in_survives_the_new_grant() {
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf());
    let saved = store.save(&connection()).unwrap();
    store.require_scope(&saved, "deploy").unwrap();
    let current = store.list().unwrap().remove(0);
    let (mut signed, guard) = store.prepare_sign_in(&saved, Some(&saved)).unwrap();
    signed.oauth.as_mut().unwrap().access_token = "fixture-new-grant".into();
    signed.pending_scope = None;
    store.require_scope(&current, "admin").unwrap();

    let committed = store.commit_sign_in(&signed, &guard.unwrap()).unwrap();
    assert_eq!(committed.pending_scope.as_deref(), Some("admin"));
    assert_eq!(
        store.list().unwrap()[0].pending_scope,
        committed.pending_scope
    );
}

#[test]
fn sign_in_commit_allows_refresh_but_rejects_removed_or_changed_source() {
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf());
    let saved = store.save(&connection()).unwrap();
    let (mut signed, guard) = store.prepare_sign_in(&saved, Some(&saved)).unwrap();
    let guard = guard.unwrap();
    signed.oauth.as_mut().unwrap().access_token = "fixture-new-grant".into();

    let mut refreshed = saved.clone();
    refreshed.oauth.as_mut().unwrap().access_token = "fixture-refreshed".into();
    store.save(&refreshed).unwrap();
    store.commit_sign_in(&signed, &guard).unwrap();

    let current = store.list().unwrap().remove(0);
    let (_, guard) = store.prepare_sign_in(&current, Some(&current)).unwrap();
    let guard = guard.unwrap();
    store.set_enabled(&current.id, false).unwrap();
    assert!(store.commit_sign_in(&signed, &guard).is_err());
    assert!(!store.list().unwrap()[0].enabled);

    store.set_enabled(&current.id, true).unwrap();
    let current = store.list().unwrap().remove(0);
    let (_, guard) = store.prepare_sign_in(&current, Some(&current)).unwrap();
    let guard = guard.unwrap();
    let mut changed = current.clone();
    changed.endpoint = "http://127.0.0.1:9/other".into();
    changed.oauth = None;
    store.save(&changed).unwrap();
    assert!(store.commit_sign_in(&signed, &guard).is_err());
    assert_eq!(store.list().unwrap()[0].endpoint, changed.endpoint);

    let (_, guard) = store.prepare_sign_in(&changed, Some(&changed)).unwrap();
    let guard = guard.unwrap();
    store.remove(&changed.id).unwrap();
    assert!(store.commit_sign_in(&signed, &guard).is_err());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn changing_endpoint_does_not_carry_a_late_challenge_from_the_old_server() {
    let home = Directory::new();
    let store = Store::new(home.path().to_path_buf());
    let saved = store.save(&connection()).unwrap();
    let mut draft = saved.clone();
    draft.endpoint = "http://127.0.0.1:9/new".into();
    draft.oauth = None;
    draft.pending_scope = None;
    let (mut prepared, guard) = store.prepare_sign_in(&draft, Some(&saved)).unwrap();
    assert_eq!(prepared.endpoint, draft.endpoint);
    assert_eq!(prepared.pending_scope, None);
    store.require_scope(&saved, "old-server:write").unwrap();
    // This is the new grant returned for B after the old A challenge arrived.
    prepared.oauth = Some(OAuthTokens {
        endpoint: draft.endpoint.clone(),
        access_token: "new-server-access".into(),
        refresh_token: None,
        expires_at: Some(4_000_000_000),
        token_endpoint: "http://127.0.0.1:9/new-token".into(),
        client_id: "new-server-client".into(),
        client_auth_method: ClientAuthMethod::None,
        client_secret: None,
        client_secret_expires_at: None,
        issuer: "http://127.0.0.1:9".into(),
        scope: None,
        resource: Some(draft.endpoint.clone()),
        github: false,
    });
    let committed = store.commit_sign_in(&prepared, &guard.unwrap()).unwrap();
    assert_eq!(committed.endpoint, draft.endpoint);
    assert_eq!(committed.pending_scope, None);
}
