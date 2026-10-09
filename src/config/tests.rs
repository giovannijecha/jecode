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
