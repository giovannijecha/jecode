use super::*;

pub(super) fn fixture(scenario: &str) {
    let path = std::path::PathBuf::from(std::env::var_os("JECODE_NATIVE_DIR").unwrap());
    let store = Store::new(path);
    if scenario == "startup-fallback-new" {
        store
            .save(&Settings::new("isolated-fixture-key".into(), "fixture/fallback".into()).unwrap())
            .unwrap();
    }
    if !matches!(scenario, "startup-fresh" | "startup-unconfigured-prompt") {
        let mut settings =
            Settings::selected("openai", String::new(), "fixture/model".into()).unwrap();
        if matches!(scenario, "startup-fallback" | "startup-fallback-new") {
            settings
                .credentials
                .insert("openrouter".into(), "isolated-fixture-key".into());
        } else if matches!(scenario, "startup-key-removed" | "startup-setup-empty") {
            settings
                .select_backend("openrouter", String::new())
                .unwrap();
        }
        store.save(&settings).unwrap();
    }
    let saved = std::fs::read(store.path()).ok();
    if scenario == "startup-setup-empty" {
        crate::bootstrap::configure().unwrap();
    } else {
        crate::bootstrap::run(None, None, vec![], scenario == "startup-plain").unwrap();
    }
    if matches!(scenario, "startup-fresh" | "startup-unconfigured-prompt") {
        assert!(!store.path().exists());
    }
    assert_eq!(std::fs::read(store.path()).ok(), saved);
    if scenario == "startup-unconfigured-prompt" {
        let sessions = crate::sessions::Store::new(
            store.path().parent().unwrap().to_path_buf(),
            &std::env::current_dir().unwrap(),
        )
        .unwrap();
        assert!(sessions.list().unwrap().sessions.is_empty());
    }
    println!("NATIVE_STARTUP_RESTORED");
}
