use super::*;
use crate::session::{
    End,
    history::{Receipt, Step},
};
use std::{io::Write, time::Instant};

#[test]
fn three_hundred_turns_resume_with_bounded_old_page() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    for index in 0..300 {
        history.begin(format!("prompt {index:03}")).unwrap();
        let turn = history.turns.last_mut().unwrap();
        turn.end = Some(End::Complete);
        turn.outcome = "Complete".into();
        history.checkpoint().unwrap();
    }
    assert_eq!(history.turn_count(), 300);
    history.projection.through = 290;
    history.projection.summary = "The first 290 turns completed.".into();
    history.checkpoint().unwrap();
    history.release_projected();
    assert_eq!(history.turns.len(), 10);
    drop(history);
    let saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(saved.turns, 300);
    assert_eq!(saved.history.base_turn, 290);
    assert_eq!(saved.history.turns.len(), 10);
    assert_eq!(saved.history.turns[0].prompt, "prompt 290");
    let restored = saved.history.transcript();
    assert!(restored.len() < 40);
    assert_eq!(
        restored
            .iter()
            .filter(|item| item.text == "prompt 290")
            .count(),
        1
    );
    assert!(restored[0].text.contains("Earlier canonical history"));
    let old = page(saved.history.record.as_ref().unwrap(), 1, 2).unwrap();
    assert_eq!(old[0].prompt, "prompt 001");
    assert_eq!(old[1].prompt, "prompt 002");
    let transcript = saved.transcript_page(1, 2).unwrap();
    assert!(transcript.iter().any(|item| item.text == "prompt 001"));
}

#[test]
fn canonical_log_exceeds_old_snapshot_limit_and_older_receipts_are_exact() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let started = Instant::now();
    let mut history = create(&store, Model::Terra, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    for index in 0..22 {
        history.begin(format!("read marker {index:02}")).unwrap();
        let call = crate::session::tool_tests::call(
            &format!("read-{index:02}"),
            "read_file",
            "{\"path\":\"a.txt\"}",
        );
        let turn = history.turns.last_mut().unwrap();
        turn.steps.push(Step {
            response: Some(crate::session::tool_tests::calls_response(vec![call])),
            accepted: true,
            results: vec![Receipt {
                call_id: format!("read-{index:02}"),
                output: format!("MARKER-{index:02} {}", "x".repeat(800_000)),
                summary: format!("Read marker {index:02}"),
                image: None,
            }],
            ..Default::default()
        });
        turn.end = Some(End::Complete);
        turn.outcome = "Complete".into();
        history.checkpoint().unwrap();
        history.projection.through = history.turns.len();
        history.projection.summary = format!("Through marker {index:02}");
        history.checkpoint().unwrap();
        history.release_projected();
    }
    let bytes = std::fs::metadata(
        store
            .directory("sessions-v2")
            .unwrap()
            .root()
            .join(format!("{id}.log")),
    )
    .unwrap()
    .len();
    eprintln!(
        "v2 fixture: turns=22 log_bytes={bytes} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    assert!(bytes > 16 * 1024 * 1024);
    assert!(history.turns.is_empty());
    drop(history);
    let saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(saved.turns, 22);
    assert!(saved.history.turns.is_empty());
    let record = saved.history.record.as_ref().unwrap();
    let first = page(record, 0, 1).unwrap();
    let last = page(record, 21, 1).unwrap();
    assert!(first[0].steps[0].results[0].output.starts_with("MARKER-00"));
    assert!(last[0].steps[0].results[0].output.starts_with("MARKER-21"));
    assert_eq!(last[0].steps[0].results[0].call_id, "read-21");
}

#[test]
fn incomplete_tail_recovers_but_committed_corruption_is_reported() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("durable prompt".into()).unwrap();
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    drop(history);
    let path = store
        .directory("sessions-v2")
        .unwrap()
        .root()
        .join(format!("{id}.log"));
    let committed = std::fs::metadata(&path).unwrap().len();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"\x12\x00\x00")
        .unwrap();
    let mut saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(saved.history.turns[0].prompt, "durable prompt");
    saved.history.begin("second prompt".into()).unwrap();
    saved.history.checkpoint().unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > committed);
    drop(saved);
    let mut bytes = std::fs::read(&path).unwrap();
    let original = bytes.clone();
    bytes[25] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    let error = super::super::load(&store, &id, true).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        error
            .to_string()
            .contains("committed session log is corrupt")
    );
    std::fs::write(&path, original).unwrap();
    let head = store
        .directory("sessions-v2")
        .unwrap()
        .root()
        .join(format!("{id}.head"));
    let mut contents = std::fs::read(&head).unwrap();
    let marker = contents
        .windows(6)
        .position(|window| window == b"second")
        .unwrap();
    contents[marker] = b'S';
    std::fs::write(&head, contents).unwrap();
    let error = super::super::load(&store, &id, true).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        error
            .to_string()
            .contains("committed session head is corrupt")
    );
    std::fs::write(&head, vec![b'x'; head::HEAD_LIMIT + 1]).unwrap();
    let error = super::super::load(&store, &id, true).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        error
            .to_string()
            .contains("committed session head is corrupt")
    );
}

#[test]
fn uncertain_head_ack_refuses_same_owner_retry_and_resumes_committed_prefix() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history
        .begin("committed despite lost acknowledgement".into())
        .unwrap();
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    let tracker = history
        .record
        .as_ref()
        .unwrap()
        .incremental
        .as_ref()
        .unwrap();
    tracker.lock().unwrap().fail_after_head_replace = true;
    assert!(matches!(
        history.checkpoint(),
        Err(crate::session::Failure::Storage)
    ));
    assert!(tracker.lock().unwrap().uncertain_commit);
    assert!(matches!(
        history.checkpoint(),
        Err(crate::session::Failure::Storage)
    ));
    drop(history);
    let mut saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(saved.turns, 1);
    assert_eq!(
        saved.history.turns[0].prompt,
        "committed despite lost acknowledgement"
    );
    saved.history.begin("safe continuation".into()).unwrap();
    saved.history.checkpoint().unwrap();
    assert_eq!(saved.history.turn_count(), 2);
}

#[test]
fn long_single_turn_releases_completed_steps_without_losing_canonical_order() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("many steps".into()).unwrap();
    for index in 0..29 {
        let turn = history.turns.first_mut().unwrap();
        if index % 5 == 0 {
            turn.guidance.push(crate::session::queue::Guidance {
                after_step: turn.steps.len(),
                text: format!("guidance {index}"),
            });
        }
        turn.steps.push(Step {
            text: format!("answer {index}"),
            ..Default::default()
        });
        history.checkpoint().unwrap();
        history.projection.step = history.turns[0].steps.len();
        history.projection.summary = format!("through step {index}");
        history.checkpoint().unwrap();
        history.release_projected();
        assert!(history.turns[0].steps.is_empty());
    }
    assert_eq!(history.base_step, 29);
    history.turns[0].steps.push(Step {
        text: "last answer".into(),
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    drop(history);
    let saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(saved.history.base_step, 29);
    assert_eq!(saved.history.turns[0].steps.len(), 1);
    assert_eq!(saved.history.turns[0].steps[0].text, "last answer");
    let full = page(saved.history.record.as_ref().unwrap(), 0, 1).unwrap();
    assert_eq!(full[0].steps.len(), 30);
    assert_eq!(full[0].steps[0].text, "answer 0");
    assert_eq!(full[0].steps[29].text, "last answer");
    assert_eq!(full[0].guidance.len(), 6);
}

#[test]
fn explicit_v1_import_verifies_canonical_copy_and_preserves_source() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let directory = crate::session::scope::Directory::open(&fixture.0).unwrap();
    let selected = Model::Luna.with_effort(Some("high")).unwrap();
    let mut legacy =
        super::super::create_legacy_in(&store, selected, Some(directory.path()), None).unwrap();
    let old_id = legacy.record.as_ref().unwrap().id().to_owned();
    legacy.begin("original task".into()).unwrap();
    let call = crate::session::tool_tests::call("read-one", "read_file", "{\"path\":\"one.txt\"}");
    legacy.turns[0].steps.push(Step {
        response: Some(crate::session::tool_tests::calls_response(vec![call])),
        accepted: true,
        results: vec![Receipt {
            call_id: "read-one".into(),
            output: "exact receipt marker".into(),
            summary: "Read one".into(),
            image: None,
        }],
        ..Default::default()
    });
    legacy.turns[0].end = Some(End::Complete);
    legacy.turns[0].outcome = "Complete".into();
    legacy.projection.through = 1;
    legacy.projection.summary = "Read one file".into();
    legacy.checkpoint().unwrap();
    drop(legacy);
    let source_path = store
        .directory("sessions")
        .unwrap()
        .root()
        .join(format!("{old_id}.json"));
    let before = std::fs::read(&source_path).unwrap();
    let new_id = super::super::import_in_store(&store, &old_id, &directory).unwrap();
    assert_ne!(new_id, old_id);
    assert_eq!(std::fs::read(&source_path).unwrap(), before);
    let imported = super::super::load(&store, &new_id, true).unwrap();
    assert_eq!(imported.model, selected);
    assert_eq!(imported.turns, 1);
    assert_eq!(imported.history.projection.summary, "Read one file");
    let first = page(imported.history.record.as_ref().unwrap(), 0, 1).unwrap();
    assert_eq!(first[0].steps[0].results[0].output, "exact receipt marker");
    drop(imported);
    let legacy_again = super::super::load(&store, &old_id, true).unwrap();
    assert_eq!(
        legacy_again.history.turns[0].steps[0].results[0].output,
        "exact receipt marker"
    );
}

#[test]
fn failed_pre_and_post_effect_commits_stop_later_effects_and_resume_as_unknown() {
    use crate::{
        providers::openai_account::{Progress, Request, Response, client},
        session::{self, Event, Failure, Session},
        tls::Budget,
    };
    use std::ops::ControlFlow;
    struct TwoEffects;
    impl session::worker::Backend for TwoEffects {
        fn login(
            &mut self,
            _: &Budget<'_>,
            _: &mut dyn FnMut(&str) -> ControlFlow<()>,
        ) -> Result<(), client::Error> {
            Ok(())
        }
        fn generate(
            &mut self,
            _: &Request,
            _: &Budget<'_>,
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            Ok(session::tool_tests::calls_response(vec![
                session::tool_tests::call(
                    "create-first",
                    "create_file",
                    r#"{"path":"first.txt","content":"first"}"#,
                ),
                session::tool_tests::call(
                    "create-second",
                    "create_file",
                    r#"{"path":"second.txt","content":"second"}"#,
                ),
            ]))
        }
    }
    for (failure_call, expected_proposals) in [(4, 0), (5, 1)] {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else { return };
        let workspace = Workspace::open(&fixture.0).unwrap();
        let history = create(&store, Model::Luna, Some(&fixture.0), Some(&workspace)).unwrap();
        let id = history.record.as_ref().unwrap().id().to_owned();
        history
            .fail_checkpoint_from
            .store(failure_call, Ordering::Release);
        let mut run =
            Session::with_history(Model::Luna, TwoEffects, Some(workspace), history).unwrap();
        assert!(matches!(
            session::tests::next(&mut run),
            Event::Restored { .. }
        ));
        assert!(matches!(session::tests::next(&mut run), Event::Ready));
        assert!(run.submit("make two files"));
        let mut proposals = 0;
        let mut completions = Vec::new();
        loop {
            match session::tests::next(&mut run) {
                Event::EditPlanned { .. } => {
                    proposals += 1;
                }
                Event::EditFinished {
                    applied, failed, ..
                } => completions.push((applied, failed)),
                Event::Finished(End::Failed(Failure::Storage), _) => break,
                Event::Text(_) => {}
                _ => panic!("unexpected event before storage failure"),
            }
        }
        assert_eq!(proposals, expected_proposals);
        assert_eq!(
            completions,
            if expected_proposals == 1 {
                vec![(true, false)]
            } else {
                Vec::new()
            }
        );
        assert!(!run.submit("later work"));
        drop(run);
        assert_eq!(
            fixture.0.join("first.txt").exists(),
            expected_proposals == 1
        );
        assert!(!fixture.0.join("second.txt").exists());
        let saved = super::super::load(&store, &id, true).unwrap();
        let receipt = &saved.history.turns[0].steps[0].results[0];
        if expected_proposals == 1 {
            assert!(receipt.summary.contains("outcome unknown"));
        } else {
            assert_eq!(receipt.summary, "Not executed");
        }
    }
}
