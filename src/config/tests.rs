use super::*;
use crate::test_support::Directory;
use std::fs;

fn settings(model: &str) -> Settings {
    Settings::new("isolated-fixture-key".into(), model.into()).unwrap()
}

#[test]
fn stores_user_configuration_and_preserves_future_fields() {
    let directory = Directory::new();
    let store = Store::new(directory.path().join(".jecode"));
    assert!(store.load().unwrap().is_none());
    store.save(&settings("fixture/first")).unwrap();
    let mut document = crate::json::parse(&fs::read_to_string(store.path()).unwrap()).unwrap();
    let Value::Object(root) = &mut document else {
        panic!()
    };
    root.insert("future".into(), Value::Bool(true));
    let Value::Object(provider) = root.get_mut("openrouter").unwrap() else {
        panic!()
    };
    provider.insert("future_option".into(), Value::number(7));
    fs::write(store.path(), document.encode()).unwrap();
    store.save(&settings("fixture/second")).unwrap();
    let reopened = Store::new(directory.path().join(".jecode"));
    let saved = reopened.load().unwrap().unwrap();
    assert_eq!(saved.api_key, "isolated-fixture-key");
    assert_eq!(saved.model, "fixture/second");
    let document = crate::json::parse(&fs::read_to_string(store.path()).unwrap()).unwrap();
    assert_eq!(document.get("future"), Some(&Value::Bool(true)));
    assert_eq!(
        document.get("openrouter").unwrap().get("future_option"),
        Some(&Value::number(7))
    );
    assert_eq!(
        fs::read_dir(store.path().parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn rejects_corrupt_incomplete_and_oversized_files_without_replacing_them() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    for original in [
        "{broken".to_string(),
        "{}".to_string(),
        "[]".to_string(),
        "x".repeat(65 * 1024),
        r#"{"openrouter":{"api_key":"private fixture secret","model":"fixture/model"}}"#
            .to_string(),
    ] {
        fs::write(store.path(), &original).unwrap();
        let error = store.load().err().unwrap();
        assert!(!error.contains("private fixture secret"));
        assert!(store.save(&settings("fixture/new")).is_err());
        assert_eq!(fs::read_to_string(store.path()).unwrap(), original);
    }
}

#[test]
fn readonly_configuration_is_preserved() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    store.save(&settings("fixture/original")).unwrap();
    let original = fs::read(store.path()).unwrap();
    let permissions = fs::metadata(store.path()).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(store.path(), readonly).unwrap();
    assert!(store.save(&settings("fixture/new")).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), original);
    fs::set_permissions(store.path(), permissions).unwrap();
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn config_paths_are_user_scoped_and_environment_pairs_are_explicit() {
    let directory_fixture = Directory::new();
    let user_home = directory_fixture.path().to_path_buf();
    assert_eq!(
        directory(None, Some(user_home.clone())).unwrap(),
        user_home.join(".jecode")
    );
    assert!(directory(Some(PathBuf::from("relative")), Some(user_home)).is_err());
    assert!(directory(None, None).is_err());
    assert!(environment_settings(None, None).unwrap().is_none());
    assert!(environment_settings(Some("isolated-fixture-key".into()), None).is_err());
    assert!(
        environment_settings(
            Some("isolated-fixture-key".into()),
            Some("fixture/model".into())
        )
        .unwrap()
        .is_some()
    );
}

#[test]
fn auto_compact_defaults_persists_beside_the_provider_and_rejects_invalid_values() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let legacy = r#"{"openrouter":{"api_key":"isolated-fixture-key","model":"fixture/model"},"context":{"future":1}}"#;
    fs::write(store.path(), legacy).unwrap();
    let mut saved = store.load().unwrap().unwrap();
    assert_eq!(saved.auto_compact, crate::context::AUTO_COMPACT_DEFAULT);
    saved.auto_compact = 65;
    store.save(&saved).unwrap();
    let document = crate::json::parse(&fs::read_to_string(store.path()).unwrap()).unwrap();
    let context = document.get("context").unwrap();
    assert_eq!(
        context.get("auto_compact_percent"),
        Some(&Value::number(65))
    );
    assert_eq!(context.get("future"), Some(&Value::number(1)));
    assert!(
        document
            .get("openrouter")
            .unwrap()
            .get("auto_compact_percent")
            .is_none()
    );
    assert_eq!(store.load().unwrap().unwrap().auto_compact, 65);
    let original = fs::read(store.path()).unwrap();
    for invalid in [0, 100] {
        saved.auto_compact = invalid;
        assert!(store.save(&saved).unwrap_err().contains("1 to 99"));
        assert_eq!(fs::read(store.path()).unwrap(), original);
    }
    for value in ["0", "100", "\"80\"", "80.5", "-1", "null"] {
        let text = format!(
            r#"{{"openrouter":{{"api_key":"isolated-fixture-key","model":"fixture/model"}},"context":{{"auto_compact_percent":{value}}}}}"#
        );
        fs::write(store.path(), &text).unwrap();
        let error = store.load().err().unwrap();
        assert!(error.contains("context.auto_compact_percent"), "{error}");
        assert!(!error.contains("isolated-fixture-key"));
        assert!(store.save(&settings("fixture/new")).is_err());
        assert_eq!(fs::read_to_string(store.path()).unwrap(), text);
    }
    let text =
        r#"{"openrouter":{"api_key":"isolated-fixture-key","model":"fixture/model"},"context":80}"#;
    fs::write(store.path(), text).unwrap();
    assert!(
        store
            .load()
            .err()
            .unwrap()
            .contains("context must be an object")
    );
    assert!(store.save(&settings("fixture/new")).is_err());
    assert_eq!(fs::read_to_string(store.path()).unwrap(), text);
}

#[test]
fn routing_preferences_are_optional_per_model_and_keep_entries_this_version_skips() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    // Files written before routing existed load with Auto for every model.
    fs::write(
        store.path(),
        r#"{"openrouter":{"api_key":"isolated-fixture-key","model":"fixture/model"}}"#,
    )
    .unwrap();
    let loaded = store.load().unwrap().unwrap();
    assert!(loaded.routing.is_empty());
    assert_eq!(loaded.routing_for("fixture/model"), Routing::Auto);
    fs::write(
        store.path(),
        r#"{"openrouter":{"api_key":"isolated-fixture-key","model":"fixture/model","routing":{
            "fixture/model":{"mode":"endpoint","endpoint":"deepinfra/fp8"},
            "fixture/other":{"mode":"auto"},
            "fixture/future":{"mode":"fastest","margin":2},
            "fixture/legacy":{"mode":"default"},
            "not a model":{"mode":"auto"}}}}"#,
    )
    .unwrap();
    let mut loaded = store.load().unwrap().unwrap();
    assert_eq!(
        loaded.routing_for("fixture/model"),
        Routing::Endpoint("deepinfra/fp8".into())
    );
    assert_eq!(loaded.routing.len(), 2);
    assert_eq!(loaded.routing_for("fixture/future"), Routing::Auto);
    assert_eq!(loaded.routing_for("fixture/legacy"), Routing::Auto);
    loaded.routing.insert("fixture/model".into(), Routing::Auto);
    loaded
        .routing
        .insert("fixture/third".into(), Routing::Endpoint("groq".into()));
    store.save(&loaded).unwrap();
    let document = crate::json::parse(&fs::read_to_string(store.path()).unwrap()).unwrap();
    let routing = document.get("openrouter").unwrap().get("routing").unwrap();
    assert_eq!(
        routing.get("fixture/model").unwrap().encode(),
        r#"{"mode":"auto"}"#
    );
    assert_eq!(
        routing.get("fixture/third").unwrap().encode(),
        r#"{"endpoint":"groq","mode":"endpoint"}"#
    );
    assert_eq!(
        routing.get("fixture/future").unwrap().encode(),
        r#"{"margin":2,"mode":"fastest"}"#
    );
    assert!(routing.get("not a model").is_some());
    let reloaded = store.load().unwrap().unwrap();
    assert_eq!(
        reloaded.routing_for("fixture/third"),
        Routing::Endpoint("groq".into())
    );
    // Saving without routing preferences leaves the file's entries alone.
    store.save(&settings("fixture/model")).unwrap();
    assert_eq!(store.load().unwrap().unwrap().routing.len(), 3);
}

#[test]
fn malformed_routing_rejects_the_file_without_replacing_it() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let text = r#"{"openrouter":{"api_key":"isolated-fixture-key","model":"fixture/model","routing":["auto"]}}"#;
    fs::write(store.path(), text).unwrap();
    let error = store.load().err().unwrap();
    assert!(error.contains("openrouter.routing must be an object"));
    assert!(!error.contains("isolated-fixture-key"));
    let mut next = settings("fixture/model");
    next.routing.insert("fixture/model".into(), Routing::Auto);
    assert!(store.save(&next).is_err());
    assert_eq!(fs::read_to_string(store.path()).unwrap(), text);
}
