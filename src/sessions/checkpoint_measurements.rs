//! Opt-in checkpoint latency with synthetic history and isolated storage.
use super::*;
use crate::{effort::Effort, json::Value, test_support::Directory};
use std::time::Instant;

#[test]
#[ignore = "release disk benchmark; no network or user configuration"]
fn active_turn_checkpoints() {
    let home = Directory::new();
    let project = Directory::new();
    let store = Store::new(home.path().into(), project.path()).unwrap();
    let mut document = Document::new(
        project.path().into(),
        "fixture/model".into(),
        Effort::Default,
    )
    .unwrap();
    document.messages = (0..400)
        .map(|index| {
            Value::object([
                (
                    "role",
                    Value::string(if index == 0 {
                        "system"
                    } else if index % 2 == 0 {
                        "assistant"
                    } else {
                        "user"
                    }),
                ),
                ("content", Value::string("x".repeat(4096))),
            ])
        })
        .collect();
    document.pending.active = true;
    let lease = store.create(&document.id).unwrap();
    lease.save(&document).unwrap();
    let mut samples = Vec::new();
    for index in 0..24 {
        document
            .pending
            .partial
            .push_str("Generated text fragment. ");
        let start = Instant::now();
        lease.save(&document).unwrap();
        let elapsed = start.elapsed().as_micros();
        if index >= 3 {
            samples.push(elapsed);
        }
    }
    let saved = store.fixture_load(&document.id).unwrap();
    assert_eq!(saved.messages, document.messages);
    assert_eq!(saved.pending.partial, document.pending.partial);
    println!("active checkpoints: 400 × 4 KiB messages, 3 warmups, 21 samples (µs): {samples:?}");
    samples.sort_unstable();
    println!(
        "median={} µs range={}–{} µs",
        samples[10], samples[0], samples[20]
    );
}
