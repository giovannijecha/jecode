//! Isolated child of the native ConPTY latency measurement. Only the parent
//! host times input delivery through the real TUI to its observed VT output.
use crate::agent::Agent;
use crate::backend::openrouter::OpenRouter;
use crate::checkpoints::session::Session as CheckpointSession;
use crate::config::{Settings, Store};
use crate::session::SessionConfig;
use crate::test_support::HttpFixture;
use crate::tools::Tools;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(super) fn fixture(path: &Path, bash: &Path, active: bool) {
    let home = path.join("home");
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let config = SessionConfig {
        store: Store::new(home.clone()),
        settings,
        bash: bash.to_path_buf(),
    };
    config.store.save(&config.settings).unwrap();
    let work = active.then(|| {
        let file = path.join("latency-work.bin");
        let first = vec![b'a'; 1 << 20];
        let second = vec![b'b'; 1 << 20];
        std::fs::write(&file, &first).unwrap();
        let file = std::fs::canonicalize(file).unwrap();
        (file, first, second)
    });
    let provider = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(provider.endpoint.clone()),
        Tools::new(path).unwrap(),
    );
    agent.enable_checkpoints(&home);

    let stop = Arc::new(AtomicBool::new(false));
    let writer = work.map(|(file, first, second)| {
        let session = CheckpointSession::open(&home, path);
        session.begin("latency-writer");
        let counter = home.join("latency-count.txt");
        std::fs::write(&counter, "0").unwrap();
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let started = Instant::now();
            let cancellation = crate::cancel::Cancellation::default();
            let mut count = 0;
            while !stop.load(Ordering::Relaxed)
                && count < 400
                && started.elapsed() < Duration::from_secs(20)
            {
                let content = if count % 2 == 0 { &second } else { &first };
                let mut guard = session.file_cancellable("write", &file, &cancellation);
                guard.capture();
                std::fs::write(&file, content).unwrap();
                let result = guard.finish();
                assert_eq!(
                    result.get("status").and_then(crate::json::Value::as_str),
                    Some("saved"),
                    "{result:?}"
                );
                count += 1;
                std::fs::write(&counter, count.to_string()).unwrap();
                std::thread::sleep(Duration::from_millis(20));
            }
            count
        })
    });

    let result = crate::tui::run(agent, config, Default::default());
    stop.store(true, Ordering::Relaxed);
    if let Some(writer) = writer {
        let count = writer.join().unwrap();
        assert!(count > 0, "active checkpoint writer made no changes");
    }
    result.unwrap();
    assert!(
        provider.finish().is_empty(),
        "latency fixture unexpectedly called a provider"
    );
}
