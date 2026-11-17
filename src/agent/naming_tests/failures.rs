use super::*;
use crate::{
    attachments::Prompt,
    json::Value,
    sessions::{Draft, Input},
};

fn fail_setup_save_once(fixture: &Fixture) {
    let agent = fixture.agent.as_ref().unwrap();
    let handle = agent.sessions().unwrap();
    let journal = handle
        .store()
        .session_directory(&handle.id())
        .join("session.jsonl");
    let retained = journal.with_extension("retained");
    let (from, to) = (journal.clone(), retained.clone());
    *fixture.gate.before_create.lock().unwrap() = Some(Box::new(move || {
        std::fs::rename(&from, &to).unwrap();
        std::fs::create_dir(&from).unwrap();
    }));
    *fixture.gate.before_main_request.lock().unwrap() = Some(Box::new(move || {
        std::fs::remove_dir(&journal).unwrap();
        std::fs::rename(&retained, &journal).unwrap();
    }));
}

fn unselected_draft(fixture: &Fixture) -> Document {
    let mut agent = Agent::with_backend(
        crate::backend::unconfigured::client(
            "openrouter",
            None,
            None,
            "Fixture needs a provider".into(),
        ),
        Tools::new(fixture._project.path()).unwrap(),
    );
    agent.enable_sessions(fixture._home.path()).unwrap();
    let path = fixture._project.path().join("draft.txt");
    std::fs::write(&path, "Keep this attachment").unwrap();
    let attachments = agent.import_attachments(&[path]).unwrap();
    let handle = agent.sessions().unwrap();
    handle.input(Input {
        draft: Draft::from_prompt(Prompt::new("Keep this draft".into(), attachments)),
        ..Input::default()
    });
    agent.save_session().unwrap();
    handle.snapshot()
}

#[test]
fn a_title_error_cannot_select_a_draft_when_resume_fails() {
    let mut fixture = Fixture::new(TitleOutcome::CreateFailure);
    let target = unselected_draft(&fixture);
    assert!(target.model.is_none());
    fail_setup_save_once(&fixture);
    let agent = fixture.agent.as_mut().unwrap();
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    let handle = agent.sessions().unwrap();
    let old_id = handle.id();
    let store = handle.store();
    drop(handle);
    let error = agent.resume(&target.id).unwrap_err();
    assert!(error.contains("regular files"), "{error}");
    assert_eq!(agent.sessions().unwrap().id(), old_id);
    let after = store.read(&target.id).unwrap();
    assert_eq!(after.model, target.model);
    assert_eq!(after.backend, target.backend);
    assert_eq!(after.effort, target.effort);
    assert_eq!(after.input, target.input);
    assert_eq!(after.messages, target.messages);
    assert_eq!(after.events, target.events);
    agent.resume(&target.id).unwrap();
    let current = agent.sessions().unwrap();
    assert_eq!(current.id(), target.id);
    assert_eq!(
        current.snapshot().model.as_deref(),
        Some("shared-model-name")
    );
    assert_eq!(current.snapshot().input, target.input);
    let (old, _) = Handle::open(store, &old_id, Redactor::empty()).unwrap();
    assert_eq!(
        title_event(&old.snapshot())
            .get("status")
            .and_then(Value::as_str),
        Some("failed")
    );
}

fn block_title_save_until_backend_drop(fixture: &Fixture, agent: &Agent) {
    let handle = agent.sessions().unwrap();
    let journal = handle
        .store()
        .session_directory(&handle.id())
        .join("session.jsonl");
    let retained = journal.with_extension("retained");
    std::fs::rename(&journal, &retained).unwrap();
    std::fs::create_dir(&journal).unwrap();
    *fixture.gate.after_title.lock().unwrap() = Some(Box::new(move || {
        std::fs::remove_dir(&journal).unwrap();
        std::fs::rename(&retained, &journal).unwrap();
    }));
}

#[test]
fn interactive_exit_flushes_title_usage_after_a_late_save_failure() {
    let mut fixture = Fixture::new(TitleOutcome::Failed(Some(Usage {
        input_tokens: Some(17),
        ..Usage::default()
    })));
    let mut agent = fixture.agent.take().unwrap();
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    fixture.gate.wait_for(|state| state.entered);
    let handle = agent.sessions().unwrap();
    let store = handle.store();
    let id = handle.id();
    drop(handle);
    block_title_save_until_backend_drop(&fixture, &agent);
    crate::tui::close_fixture(
        agent,
        crate::session::SessionConfig {
            store: crate::config::Store::new(fixture._home.path().join("config.json")),
            settings: crate::config::Settings::unconfigured(),
        },
    );
    assert!(fixture.gate.state.lock().unwrap().finished);
    let saved = store.read(&id).unwrap();
    let event = title_event(&saved);
    assert_eq!(
        event.get("status").and_then(Value::as_str),
        Some("cancelled")
    );
    assert_eq!(
        event.get("input_tokens").and_then(Value::as_usize),
        Some(17)
    );
    assert_eq!(event.get("output_tokens"), Some(&Value::Null));
    let (_reopened, _) = Handle::open(store, &id, Redactor::empty()).unwrap();
}

#[test]
fn plain_exit_and_eof_finish_title_work_before_the_final_save() {
    for input in ["", "/exit\n"] {
        let mut fixture = Fixture::new(TitleOutcome::Failed(Some(Usage {
            input_tokens: Some(17),
            ..Usage::default()
        })));
        let mut agent = fixture.agent.take().unwrap();
        agent.run_turn("First request", &mut |_| Ok(())).unwrap();
        fixture.gate.wait_for(|state| state.entered);
        let handle = agent.sessions().unwrap();
        let store = handle.store();
        let id = handle.id();
        drop(handle);
        block_title_save_until_backend_drop(&fixture, &agent);
        let error = crate::session::chat(
            &mut agent,
            &mut crate::session::SessionConfig {
                store: crate::config::Store::new(fixture._home.path().join("config.json")),
                settings: crate::config::Settings::unconfigured(),
            },
            &mut std::io::Cursor::new(input),
            &mut Vec::new(),
            &mut Vec::new(),
            vec![],
        )
        .unwrap_err();
        assert!(error.contains("regular files"), "{error}");
        assert!(fixture.gate.state.lock().unwrap().finished);
        let saved = store.read(&id).unwrap();
        let event = title_event(&saved);
        assert_eq!(
            event.get("status").and_then(Value::as_str),
            Some("cancelled")
        );
        assert_eq!(
            event.get("input_tokens").and_then(Value::as_usize),
            Some(17)
        );
        assert_eq!(event.get("output_tokens"), Some(&Value::Null));
        drop(agent);
        let (_reopened, _) = Handle::open(store, &id, Redactor::empty()).unwrap();
    }
}

#[test]
fn title_events_redact_model_and_error_before_entering_memory() {
    const MODEL_SECRET: &str = "isolated-model-secret";
    const ERROR_SECRET: &str = "fixture title";
    for outcome in [
        TitleOutcome::Complete(None),
        TitleOutcome::Failed(None),
        TitleOutcome::PrepareFailure,
        TitleOutcome::CreateFailure,
    ] {
        let mut fixture = Fixture::with_model(outcome, &format!("fixture/{MODEL_SECRET}"));
        let gate = fixture.gate.clone();
        let agent = fixture.agent.as_mut().unwrap();
        agent.include_credentials(Redactor::new(MODEL_SECRET.into()));
        agent.include_credentials(Redactor::new(ERROR_SECRET.into()));
        agent
            .run_single_turn("First request", &mut |event| {
                if matches!(event, crate::events::Event::Message { .. }) {
                    gate.release();
                }
                Ok(())
            })
            .unwrap();
        let archive = agent.archive();
        let events = archive.events.lock().unwrap();
        let event = events
            .iter()
            .find(|event| event.get("operation").and_then(Value::as_str) == Some("session_title"))
            .unwrap();
        assert_eq!(
            event.get("model").and_then(Value::as_str),
            Some("fixture/[redacted]")
        );
        let encoded = event.encode();
        assert!(!encoded.contains(MODEL_SECRET), "{encoded}");
        assert!(!encoded.contains(ERROR_SECRET), "{encoded}");
        if !matches!(outcome, TitleOutcome::Complete(_)) {
            assert!(
                event
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap()
                    .contains("[redacted]")
            );
        }
        let handle = agent.sessions().unwrap();
        assert_eq!(
            title_event(&handle.store().read(&handle.id()).unwrap()),
            event
        );
    }
}

#[test]
fn a_title_save_error_blocks_new_and_delete_once_without_losing_the_session() {
    for delete in [false, true] {
        let mut fixture = Fixture::new(TitleOutcome::Failed(None));
        let mut agent = fixture.agent.take().unwrap();
        agent.run_turn("First request", &mut |_| Ok(())).unwrap();
        fixture.gate.wait_for(|state| state.entered);
        let handle = agent.sessions().unwrap();
        let store = handle.store();
        let id = handle.id();
        drop(handle);
        block_title_save_until_backend_drop(&fixture, &agent);
        fixture.gate.release();
        fixture.gate.wait_for(|state| state.finished);
        let transition = |agent: &mut Agent| {
            if delete {
                agent
                    .delete_session(&id, agent.model().into(), Effort::High)
                    .map(|_| ())
            } else {
                agent.start_new(agent.model().into(), Effort::High)
            }
        };
        let error = transition(&mut agent).unwrap_err();
        assert!(error.contains("regular files"), "{error}");
        assert_eq!(agent.sessions().unwrap().id(), id);
        assert!(!agent.cancellation().requested());
        assert!(store.read(&id).is_ok());
        transition(&mut agent).unwrap();
        assert_ne!(agent.sessions().unwrap().id(), id);
        if delete {
            assert!(store.read(&id).is_err());
        } else {
            let (old, _) = Handle::open(store, &id, Redactor::empty()).unwrap();
            assert_eq!(
                title_event(&old.snapshot())
                    .get("status")
                    .and_then(Value::as_str),
                Some("failed")
            );
        }
    }
}

#[test]
fn a_resume_without_the_required_account_keeps_the_current_title_running() {
    let mut fixture = Fixture::new(TitleOutcome::Failed(None));
    let mut other = Agent::new(
        Account::new("beta", vec![]).client(),
        Tools::new(fixture._project.path()).unwrap(),
    );
    other.enable_sessions(fixture._home.path()).unwrap();
    other.prepare_turn("Other request").unwrap();
    let target_id = other.sessions().unwrap().id();
    drop(other);

    let agent = fixture.agent.as_mut().unwrap();
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    fixture.gate.wait_for(|state| state.entered);
    let handle = agent.sessions().unwrap();
    let id = handle.id();
    let error = agent.resume(&target_id).unwrap_err();
    assert!(
        error.contains("Backend beta has no configured account"),
        "{error}"
    );
    assert_eq!(agent.sessions().unwrap().id(), id);
    assert!(!fixture.gate.state.lock().unwrap().finished);
    fixture.gate.release();
    fixture.gate.wait_for(|state| state.finished);
    agent.finish_session().unwrap();
    assert_eq!(
        title_event(&handle.store().read(&id).unwrap())
            .get("status")
            .and_then(Value::as_str),
        Some("failed")
    );
}
