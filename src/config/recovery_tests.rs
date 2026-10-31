use super::*;
use crate::{json, test_support::Directory};
use std::fs;

#[test]
fn empty_legacy_key_does_not_hide_a_scoped_key() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    fs::write(store.path(), r#"{"selection":{"backend":"openai","model":"saved-model"},"backends":{"openrouter":{"api_key":"isolated-new-key"}},"openrouter":{"api_key":"","model":"unconfigured"}}"#).unwrap();
    let settings = store.load().unwrap().unwrap();
    assert_eq!(
        settings.credential_for("openrouter"),
        Some("isolated-new-key")
    );
    assert_eq!(settings.configured_services(directory.path()).len(), 1);
}

#[test]
fn replacing_a_key_while_another_provider_is_default_updates_both_formats() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let mut settings = Settings::new("isolated-old-key".into(), "fixture/model".into()).unwrap();
    store.save(&settings).unwrap();
    settings.select_backend("openai", String::new()).unwrap();
    settings.model = "chatgpt-model".into();
    settings
        .credentials
        .insert("openrouter".into(), "isolated-new-key".into());
    store.save(&settings).unwrap();
    let saved = store.load().unwrap().unwrap();
    assert_eq!(saved.credential_for("openrouter"), Some("isolated-new-key"));
    let document = json::parse(&fs::read_to_string(store.path()).unwrap()).unwrap();
    assert_eq!(
        document
            .get("openrouter")
            .unwrap()
            .get("api_key")
            .unwrap()
            .as_str(),
        Some("isolated-new-key")
    );
    assert_eq!(saved.backend_id, "openai");
}

#[test]
fn incomplete_first_launch_state_is_never_written_as_a_default_model() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let settings = Settings::unconfigured();
    assert!(
        store
            .save(&settings)
            .unwrap_err()
            .contains("Choose a default model")
    );
    assert!(!store.path().exists());
}

#[test]
fn provider_diagnostics_and_settings_actions_are_separate() {
    let directory = Directory::new();
    let settings = Settings::unconfigured();
    let error = settings
        .service_status(directory.path(), "openrouter")
        .err()
        .unwrap();
    assert_eq!(error.diagnostic, "OpenRouter has no saved API key");
    assert_eq!(error.settings, Some("Settings > Providers > API keys"));
    assert_eq!(
        error.to_string(),
        "OpenRouter has no saved API key; open Settings > Providers > API keys"
    );
}

#[test]
fn claude_subscription_needs_an_account_and_preserves_other_provider_defaults() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let mut settings = Settings::new("isolated-router-key".into(), "router/model".into()).unwrap();
    store.save(&settings).unwrap();
    settings.select_backend("openai", String::new()).unwrap();
    settings.model = "chatgpt-model".into();
    store.save(&settings).unwrap();
    settings.select_backend("anthropic", String::new()).unwrap();
    settings.model = "claude-model".into();
    store.save(&settings).unwrap();
    let saved = store.load().unwrap().unwrap();
    assert_eq!(saved.backend_id, "anthropic");
    assert_eq!(
        saved.credential_for("openrouter"),
        Some("isolated-router-key")
    );
    assert_eq!(
        store.previous_model("openai").unwrap().as_deref(),
        Some("chatgpt-model")
    );
    assert_eq!(
        store.previous_model("anthropic").unwrap().as_deref(),
        Some("claude-model")
    );
    let error = saved
        .service_status(directory.path(), "anthropic")
        .err()
        .unwrap();
    assert_eq!(error.diagnostic, "Anthropic has no active Claude account");
    assert_eq!(error.settings, Some("Settings > Providers > Credentials"));
    crate::auth::anthropic::Store::fixture_account(
        directory.path().to_path_buf(),
        "isolated-claude-token",
    )
    .unwrap();
    let services = saved.services(directory.path(), "anthropic").unwrap();
    assert_eq!(services.id(), "anthropic");
    assert!(services.account().is_some());
    assert!(crate::backend::Services::for_backend("anthropic", "isolated-api-key".into()).is_err());
    assert_eq!(saved.configured_services(directory.path()).len(), 2);
}
