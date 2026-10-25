use super::*;

pub(super) fn fixture(scenario: &str) {
    let path = std::path::PathBuf::from(std::env::var_os("JECODE_NATIVE_DIR").unwrap());
    let store = Store::new(path);
    if scenario != "startup-fresh" {
        let mut settings =
            Settings::selected("openai", String::new(), "fixture/model".into()).unwrap();
        if scenario == "startup-fallback" {
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
    if scenario == "startup-setup-empty" {
        crate::bootstrap::configure().unwrap();
    } else {
        crate::bootstrap::run(None, None, vec![], scenario == "startup-plain").unwrap();
    }
    if scenario == "startup-fresh" {
        assert!(!store.path().exists());
    }
    println!("NATIVE_STARTUP_RESTORED");
}
