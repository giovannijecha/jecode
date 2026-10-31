use super::*;
use crate::test_support::Directory;

#[test]
fn subscriptions_have_distinct_native_credentials_and_activation() {
    let directory = Directory::new();
    let home = directory.path().to_path_buf();
    let chatgpt = openai::Store::fixture_account(home.clone(), "isolated-chatgpt-token").unwrap();
    let claude = anthropic::Store::fixture_account(home.clone(), "isolated-claude-token").unwrap();
    let openai = AccountStore::new(home.clone(), "openai");
    let anthropic = AccountStore::new(home, "anthropic");
    let openai_session = openai.active().unwrap().unwrap();
    let anthropic_session = anthropic.active().unwrap().unwrap();
    assert_eq!(openai_session.id(), chatgpt.client_id);
    assert_eq!(anthropic_session.id(), claude.id);
    assert_eq!(
        openai_session.account().unwrap().identity,
        openai_session.identity()
    );
    assert_eq!(
        anthropic_session.account().unwrap().identity,
        anthropic_session.identity()
    );
    let openai_credential = openai_session.credential();
    let anthropic_credential = anthropic_session.credential();
    assert_eq!(openai_credential.identity.backend, "openai");
    assert!(openai_credential.openai_session().is_some());
    assert!(openai_credential.anthropic_session().is_none());
    assert!(openai_credential.api_key_material().is_none());
    assert_eq!(anthropic_credential.identity.backend, "anthropic");
    assert!(anthropic_credential.anthropic_session().is_some());
    assert!(anthropic_credential.openai_session().is_none());
    assert!(anthropic_credential.api_key_material().is_none());
    anthropic.activate(&claude.id).unwrap();
    assert_eq!(openai.active().unwrap().unwrap().id(), chatgpt.client_id);
    assert_eq!(
        anthropic.redactor().unwrap().text("isolated-claude-token"),
        "[redacted]"
    );
}

#[test]
fn account_dispatch_never_accepts_an_api_key_provider() {
    let directory = Directory::new();
    let store = AccountStore::new(directory.path().to_path_buf(), "openrouter");
    assert!(!account_backend("openrouter"));
    assert!(store.accounts().is_err());
    assert!(store.active().is_err());
    assert!(store.activate("isolated-id").is_err());
    assert!(!directory.path().join("accounts").exists());
}
