use super::*;
use crate::test_support::Directory;

#[test]
fn api_keys_coexist_with_subscription_accounts_and_scoped_openrouter_settings() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let mut settings = Settings::new("router-fixture-key".into(), "fixture/router".into()).unwrap();
    store.save(&settings).unwrap();

    for (id, key, model) in [
        ("openai-api", "openai-fixture-key", "fixture-openai"),
        ("anthropic-api", "anthropic-fixture-key", "fixture-claude"),
    ] {
        settings.select_backend(id, key.into()).unwrap();
        settings.model = model.into();
        store.save(&settings).unwrap();
        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.backend_id, id);
        assert_eq!(loaded.credential_for(id), Some(key));
        assert_eq!(store.previous_model(id).unwrap().as_deref(), Some(model));
        let services = loaded.services(directory.path(), id).unwrap();
        assert_eq!(services.id(), id);
        assert!(services.account().unwrap().starts_with("key-sha256:"));
        assert!(!services.account().unwrap().contains(key));
    }

    settings.select_backend("openai", String::new()).unwrap();
    settings.model = "chatgpt-fixture-model".into();
    store.save(&settings).unwrap();
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.backend_id, "openai");
    assert!(loaded.services(directory.path(), "openai").is_err());
    assert!(loaded.services(directory.path(), "anthropic").is_err());
    assert_eq!(
        loaded.credential_for("openai-api"),
        Some("openai-fixture-key")
    );
    assert_eq!(
        loaded.credential_for("anthropic-api"),
        Some("anthropic-fixture-key")
    );
    assert_eq!(
        loaded.credential_for("openrouter"),
        Some("router-fixture-key")
    );
    assert_eq!(
        store.previous_model("openrouter").unwrap().as_deref(),
        Some("fixture/router")
    );
}

#[test]
fn rejected_api_key_does_not_change_the_saved_selection_or_expose_the_secret() {
    for id in ["openai-api", "anthropic-api"] {
        let mut settings =
            Settings::new("router-fixture-key".into(), "fixture/router".into()).unwrap();
        let key = "fixture-secret\ninvalid-header";
        let error = settings.select_backend(id, key.into()).unwrap_err();
        assert!(!error.contains("fixture-secret"));
        assert_eq!(settings.backend_id, "openrouter");
        assert_eq!(
            settings.credential_for("openrouter"),
            Some("router-fixture-key")
        );
        assert!(settings.credential_for(id).is_none());
    }
}

#[test]
fn missing_api_key_never_uses_a_saved_subscription_account() {
    let directory = Directory::new();
    crate::auth::openai::Store::fixture_account(
        directory.path().to_path_buf(),
        "isolated-chatgpt-token",
    )
    .unwrap();
    crate::auth::anthropic::Store::fixture_account(
        directory.path().to_path_buf(),
        "isolated-claude-token",
    )
    .unwrap();
    let settings = Settings::unconfigured();
    for id in ["openai-api", "anthropic-api"] {
        let error = settings.service_status(directory.path(), id).err().unwrap();
        assert!(error.diagnostic.contains("no saved API key"));
        assert_eq!(error.settings, Some("Settings > Providers > API keys"));
    }
    assert!(settings.services(directory.path(), "openai").is_ok());
    assert!(settings.services(directory.path(), "anthropic").is_ok());
}
