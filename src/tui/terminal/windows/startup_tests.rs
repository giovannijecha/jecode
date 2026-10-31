use super::*;

pub(super) fn fixture(scenario: &str) {
    let path = std::path::PathBuf::from(std::env::var_os("JECODE_NATIVE_DIR").unwrap());
    let store = Store::new(path);
    if scenario == "startup-splash" {
        std::fs::write(
            std::env::current_dir().unwrap().join("AGENTS.md"),
            "# Fixture\n",
        )
        .unwrap();
        let skill = store.path().parent().unwrap().join("skills/review");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: review\ndescription: Inspect a change.\n---\n\n# Review\n",
        )
        .unwrap();
        for title in [
            "Fix the session expiry check in auth",
            "Add the sleep test",
            "Migrate the config loader",
        ] {
            let mut agent = crate::agent::Agent::new(
                crate::backend::openrouter::OpenRouter::fixture(
                    "http://127.0.0.1:1/chat/completions".into(),
                ),
                crate::tools::Tools::new(&std::env::current_dir().unwrap()).unwrap(),
            );
            agent
                .enable_sessions(store.path().parent().unwrap())
                .unwrap();
            let archive = agent.archive();
            for (role, text) in [("user", title), ("assistant", "Saved answer")] {
                archive
                    .messages
                    .lock()
                    .unwrap()
                    .push(crate::json::Value::object([
                        ("role", crate::json::Value::string(role)),
                        ("content", crate::json::Value::string(text)),
                    ]));
            }
            agent.save_session().unwrap();
        }
        let response = crate::test_support::completion("Fixture answered.", vec![]);
        let fixture = crate::test_support::HttpFixture::new(vec![(200, response)]);
        let agent = crate::agent::Agent::new(
            crate::backend::openrouter::OpenRouter::fixture(fixture.endpoint.clone()),
            crate::tools::Tools::new(&std::env::current_dir().unwrap()).unwrap(),
        );
        let settings =
            Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
        crate::tui::launch(
            agent,
            crate::session::SessionConfig { store, settings },
            None,
            Default::default(),
            None,
        )
        .unwrap();
        assert_eq!(fixture.finish().len(), 1);
        println!("NATIVE_STARTUP_RESTORED");
        return;
    }
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
