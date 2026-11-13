//! Isolated child of the native ConPTY latency measurement. Only the parent
//! host times input delivery through the real TUI to its observed VT output.
use crate::agent::Agent;
use crate::backend::openrouter::OpenRouter;
use crate::config::{Settings, Store};
use crate::session::SessionConfig;
use crate::test_support::HttpFixture;
use crate::tools::Tools;
use std::path::Path;

pub(super) fn fixture(path: &Path) {
    let home = path.join("home");
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let config = SessionConfig {
        store: Store::new(home.clone()),
        settings,
    };
    config.store.save(&config.settings).unwrap();
    let provider = HttpFixture::new(vec![]);
    let agent = Agent::new(
        OpenRouter::fixture(provider.endpoint.clone()),
        Tools::new(path).unwrap(),
    );
    crate::tui::launch(
        agent,
        config,
        None,
        Default::default(),
        None,
        Default::default(),
    )
    .unwrap();
    assert!(
        provider.finish().is_empty(),
        "latency fixture unexpectedly called a provider"
    );
}

/// Resumes a saved mixed conversation for the native wheel measurement.
pub(super) fn scroll_fixture(path: &Path) {
    let home = path.join("home");
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let config = SessionConfig {
        store: Store::new(home.clone()),
        settings,
    };
    config.store.save(&config.settings).unwrap();
    let provider = HttpFixture::new(vec![]);
    let agent = || {
        Agent::new(
            OpenRouter::fixture(provider.endpoint.clone()),
            Tools::new(path).unwrap(),
        )
    };
    let mut saved = agent();
    saved.enable_sessions(&home).unwrap();
    saved
        .archive()
        .messages
        .lock()
        .unwrap()
        .extend(crate::tui::measurement_tests::mixed_messages(200));
    saved.save_session().unwrap();
    let id = saved.sessions().unwrap().snapshot().id.clone();
    drop(saved);
    crate::tui::launch(
        agent(),
        config,
        Some(Some(id)),
        Default::default(),
        None,
        Default::default(),
    )
    .unwrap();
    assert!(
        provider.finish().is_empty(),
        "scroll fixture unexpectedly called a provider"
    );
}
