use super::*;
use crate::{backend::Routing, effort::Effort, test_support::Directory};
use std::fs;

#[test]
fn removed_default_account_does_not_prevent_local_tui_work() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let settings = Settings::selected("openai", String::new(), "saved-model".into()).unwrap();
    store.save(&settings).unwrap();
    let original = fs::read(store.path()).unwrap();
    let (mut backend, notice) = client(&store, &settings, None, true).unwrap();
    assert_eq!(backend.id(), "openai");
    assert_eq!(backend.model(), "saved-model");
    assert!(notice.unwrap().0.contains("No configured provider"));
    assert!(backend.limits().unwrap().is_none());
    assert!(
        backend
            .prepare()
            .unwrap_err()
            .contains("No configured provider")
    );
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(client(&store, &settings, None, false).is_err());
}

#[test]
fn saved_fallback_uses_its_own_model_without_changing_defaults() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let mut settings =
        Settings::new("isolated-fixture-key".into(), "fixture/fallback".into()).unwrap();
    store.save(&settings).unwrap();
    settings.select_backend("openai", String::new()).unwrap();
    settings.model = "unavailable-model".into();
    settings.effort = Effort::High;
    store.save(&settings).unwrap();
    let original = fs::read(store.path()).unwrap();
    let (backend, notice) = client(&store, &settings, None, true).unwrap();
    assert_eq!(backend.id(), "openrouter");
    assert_eq!(backend.model(), "fixture/fallback");
    assert_eq!(backend.effort(), Effort::Default);
    assert_eq!(backend.routing(), Routing::Auto);
    assert!(notice.unwrap().0.contains("Using openrouter"));
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert_eq!(store.load().unwrap().unwrap().backend_id, "openai");
}

#[test]
fn fallback_to_chatgpt_restores_its_own_saved_model() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let mut settings = Settings::selected("openai", String::new(), "chatgpt-model".into()).unwrap();
    store.save(&settings).unwrap();
    crate::auth::openai::Store::fixture_account(
        directory.path().to_path_buf(),
        "isolated-access-token",
    )
    .unwrap();
    settings
        .select_backend("openrouter", String::new())
        .unwrap();
    settings.model = "fixture/missing".into();
    store.save(&settings).unwrap();
    let (backend, notice) = client(&store, &settings, None, true).unwrap();
    assert_eq!(backend.id(), "openai");
    assert_eq!(backend.model(), "chatgpt-model");
    assert!(backend.account().is_some());
    assert!(notice.unwrap().0.contains("Using openai"));
}

#[test]
fn fallback_without_a_saved_model_can_select_one_without_crossing_model_ids() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let mut settings = Settings::selected("openai", String::new(), "chatgpt-model".into()).unwrap();
    settings
        .credentials
        .insert("openrouter".into(), "isolated-fixture-key".into());
    store.save(&settings).unwrap();
    let (backend, notice) = client(&store, &settings, None, true).unwrap();
    assert_eq!(backend.id(), "openrouter");
    assert_eq!(backend.model(), "unconfigured");
    assert_eq!(notice.unwrap().1, "/model");
    let selected = backend.services().create("fixture/chosen".into()).unwrap();
    assert_eq!(selected.model(), "fixture/chosen");
}

#[test]
fn first_launch_needs_no_credentials_or_saved_configuration() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let settings = Settings::selected("openrouter", String::new(), "unconfigured".into()).unwrap();
    let (backend, notice) = client(&store, &settings, None, true).unwrap();
    assert_eq!(notice.unwrap().1, "/settings");
    assert!(!store.path().exists());
    assert!(backend.services().catalog().is_err());
    assert!(
        backend
            .services()
            .with_credential("isolated-fixture-key".into())
            .is_ok()
    );
    assert!(client(&store, &settings, None, false).is_err());
}

#[test]
fn an_added_credential_still_requests_a_model_after_restart() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let settings = Settings::new("isolated-fixture-key".into(), "unconfigured".into()).unwrap();
    store.save(&settings).unwrap();
    let (mut backend, notice) = client(&store, &settings, None, true).unwrap();
    assert_eq!(notice.unwrap().1, "/model");
    assert!(backend.prepare().unwrap_err().contains("Choose a model"));
    assert!(backend.services().create("fixture/model".into()).is_ok());
}

#[test]
fn configured_provider_keeps_explicit_model_overrides_and_no_notice() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/saved".into()).unwrap();
    let (backend, notice) =
        client(&store, &settings, Some("fixture/override".into()), true).unwrap();
    assert_eq!(backend.model(), "fixture/override");
    assert!(notice.is_none());
}

#[test]
fn legacy_fallback_model_remains_available() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    fs::write(store.path(), r#"{"selection":{"backend":"openai","model":"missing"},"openrouter":{"api_key":"isolated-fixture-key","model":"fixture/legacy"}}"#).unwrap();
    let settings = store.load().unwrap().unwrap();
    let (backend, _) = client(&store, &settings, None, true).unwrap();
    assert_eq!(backend.id(), "openrouter");
    assert_eq!(backend.model(), "fixture/legacy");
}
