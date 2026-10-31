//! Explicit, bounded continuity check with synthetic history and a selected Claude account.
use super::*;
use crate::auth::anthropic::Store;
use crate::backend::{Service, anthropic::Api};
use crate::effort::Effort;
use crate::test_support::Directory;
use crate::tools::Tools;
use std::time::Instant;

const MODEL: &str = "claude-opus-5-5";

fn synthetic_notes() -> String {
    let mut notes = String::new();
    for index in 0..180 {
        notes.push_str(&format!(
            "Observation {index:03}: this is synthetic continuity evidence. The recorded file paths, commands and checks are placeholders; no external changes or tools are needed.\n"
        ));
        match index {
            12 => notes.push_str(
                "JX-104: done. E_PARSE_14 was resolved. Next verify the fixture checksum.\n",
            ),
            93 => notes
                .push_str("JX-207: open. E_RETRY_06 remains. Next inspect the retry counter.\n"),
            168 => notes.push_str(
                "JX-319: open. E_SAVE_09 remains. Next restore the journal checkpoint.\n",
            ),
            _ => {}
        }
    }
    assert!((20 * 1024..=40 * 1024).contains(&notes.len()));
    notes
}

fn readback_has_facts(answer: &str) -> bool {
    let answer = answer.to_ascii_lowercase();
    let tasks = [
        ("jx-104", "done", "e_parse_14", "checksum"),
        ("jx-207", "open", "e_retry_06", "counter"),
        ("jx-319", "open", "e_save_09", "checkpoint"),
    ];
    tasks.iter().all(|(id, status, error, action)| {
        answer.lines().any(|line| {
            let words: Vec<_> = line
                .split(|character: char| {
                    !character.is_ascii_alphanumeric() && character != '-' && character != '_'
                })
                .filter(|word| !word.is_empty())
                .collect();
            words.contains(id)
                && words.contains(status)
                && words.contains(error)
                && words.contains(action)
                && tasks
                    .iter()
                    .filter(|(other, ..)| other != id)
                    .all(|(other, ..)| !words.contains(other))
        })
    })
}

#[test]
fn readback_accepts_task_facts_kept_together() {
    let correct = "JX-104: done, E_PARSE_14; verify the fixture checksum.\nJX-207: open, E_RETRY_06; inspect the retry counter.\nJX-319: open, E_SAVE_09; restore the journal checkpoint.";
    assert!(readback_has_facts(correct));
}

#[test]
fn readback_rejects_swapped_task_facts() {
    let swapped = "JX-104 open E_SAVE_09 checkpoint\nJX-207 done E_PARSE_14 checksum\nJX-319 open E_RETRY_06 counter";
    assert!(!readback_has_facts(swapped));
}

#[test]
#[ignore = "uses two bounded synthetic Claude inference requests after a usage check"]
fn live_summary_and_resumed_readback_preserve_critical_work() {
    let account_home = std::env::var_os("JECODE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .expect("Set JECODE_HOME to an isolated signed-in Jecode home");
    let session = Store::new(account_home)
        .active()
        .unwrap()
        .expect("Sign in to Claude in the selected home first");
    let api = Api::new(session).unwrap();
    assert!(
        api.extra_usage_disabled().unwrap(),
        "This bounded live check requires extra usage disabled"
    );
    api.catalog().unwrap();
    let workspace = Directory::new();
    let session_home = Directory::new();
    let mut agent = Agent::with_backend(
        api.create(MODEL.into()).unwrap(),
        Tools::new(workspace.path()).unwrap(),
    );
    agent.set_effort(Effort::High);
    agent.enable_sessions(session_home.path()).unwrap();
    agent
        .prepare_turn("Track JX-104, JX-207 and JX-319 through the inspection; retain their status, errors and next actions.")
        .unwrap();
    agent.messages.lock().unwrap().extend([
        Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::string(synthetic_notes())),
        ]),
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Continue the same task. Preserve the three IDs, completed work, open errors and concrete next actions.")),
        ]),
    ]);
    let originals = agent.messages.lock().unwrap().clone();
    let mut attempts = 0;
    let started = Instant::now();
    assert!(
        agent
            .compact_if_needed(
                Some(Limits {
                    context: 50_000,
                    output: Some(8_192),
                }),
                true,
                &mut |event| {
                    match event {
                        Event::Waiting { .. } => {
                            attempts += 1;
                            if attempts > 1 {
                                return Err("Live compaction request limit reached".into());
                            }
                        }
                        Event::Recovering { .. } | Event::RequestDiscarded => {
                            return Err("Unexpected live compaction retry".into());
                        }
                        _ => {}
                    }
                    Ok(())
                },
            )
            .unwrap()
    );
    let summary_elapsed = started.elapsed();
    assert_eq!(attempts, 1);
    assert_eq!(agent.effort(), Effort::High);
    let original_history = *agent.messages.lock().unwrap() == originals;
    assert!(original_history);
    for id in ["JX-104", "JX-207", "JX-319"] {
        assert!(agent.context.summary.contains(id));
    }
    let summary_bytes = agent.context.summary.len();
    let timing = agent
        .events
        .lock()
        .unwrap()
        .iter()
        .find(|event| event.get("operation").and_then(Value::as_str) == Some("context_compaction"))
        .cloned()
        .expect("summary timing event");
    assert_eq!(timing.get("effort").and_then(Value::as_str), Some("medium"));
    assert!(
        timing
            .get("input_tokens")
            .and_then(Value::as_usize)
            .is_some()
    );
    assert!(
        timing
            .get("output_tokens")
            .and_then(Value::as_usize)
            .is_some()
    );
    agent.save_session().unwrap();
    let handle = agent.sessions().unwrap();
    let id = handle.id();
    assert_eq!(
        handle.store().fixture_load(&id).unwrap().messages,
        originals
    );
    drop(handle);
    drop(agent);

    let mut resumed = Agent::with_backend(
        api.create(MODEL.into()).unwrap(),
        Tools::new(workspace.path()).unwrap(),
    );
    resumed.set_effort(Effort::High);
    resumed.enable_sessions(session_home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(*resumed.messages.lock().unwrap(), originals);
    assert_eq!(resumed.effort(), Effort::High);
    let mut messages = resumed.transport_messages();
    let state = messages.pop().expect("request state");
    messages.push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("For JX-104, JX-207 and JX-319, report each status, error and next action in three short lines. Use the saved context only; no tools.")),
    ]));
    messages.push(state);
    let readback_started = Instant::now();
    let readback = resumed
        .client
        .sample(&messages, false, Some(1024), None, &mut |_| Ok(()))
        .unwrap();
    let readback_elapsed = readback_started.elapsed();
    let answer = readback.text.to_ascii_lowercase();
    assert!(
        readback_has_facts(&answer),
        "Synthetic task facts were lost or mixed"
    );
    assert_eq!(*resumed.messages.lock().unwrap(), originals);
    println!(
        "summary_requests={attempts} summary_bytes={summary_bytes} summary_ms={} readback_ms={} medium_usage={} original_history={} readback_facts={}",
        summary_elapsed.as_millis(),
        readback_elapsed.as_millis(),
        timing
            .get("input_tokens")
            .and_then(Value::as_usize)
            .is_some()
            && timing
                .get("output_tokens")
                .and_then(Value::as_usize)
                .is_some(),
        original_history,
        readback_has_facts(&answer),
    );
}
