use super::*;
use crate::{
    backend::{openrouter::OpenRouter, unconfigured},
    config::{Settings, Store},
    json::Value,
    test_support::{Directory, HttpFixture, completion},
    tools::Tools,
};

fn app(directory: &Directory) -> App {
    let store = Store::new(directory.path().join("config"));
    let mut agent = Agent::with_backend(
        unconfigured::client("openrouter", None, None, "No configured provider".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent
        .enable_sessions(store.path().parent().unwrap())
        .unwrap();
    App::new(
        agent,
        SessionConfig {
            store,
            settings: Settings::unconfigured(),
            bash: crate::tools::find_bash().unwrap(),
        },
        None,
    )
}

#[test]
fn rejected_unconfigured_prompts_restore_the_draft_without_saving_a_turn() {
    let directory = Directory::new();
    let mut app = app(&directory);
    for _ in 0..2 {
        app.state
            .editor
            .set(crate::attachments::Prompt::plain("Keep my task"));
        app.submit().unwrap();
        tests::finish(&mut app);
        app.persist_input(true);
        assert_eq!(app.state.editor.text, "Keep my task");
        assert!(
            app.persistence
                .as_ref()
                .unwrap()
                .store()
                .list()
                .unwrap()
                .sessions
                .is_empty()
        );
        assert!(!app.persistence.as_ref().unwrap().snapshot().pending.active);
    }
}

#[test]
fn an_attachment_draft_has_no_model_and_resumes_with_the_current_selection() {
    for configured in [true, false] {
        resume_attachment_draft(configured);
    }
}

fn resume_attachment_draft(configured: bool) {
    let directory = Directory::new();
    let mut first = app(&directory);
    let path = directory.path().join("isolated.txt");
    std::fs::write(&path, "Keep this attachment").unwrap();
    let attachments = first
        .agent
        .as_ref()
        .unwrap()
        .import_attachments(&[path])
        .unwrap();
    let attachment = attachments[0].clone();
    first.state.editor.set(crate::attachments::Prompt::new(
        "Keep this draft".into(),
        attachments,
    ));
    let draft = first.state.editor.prompt();
    first.capture_input();
    first.persist_input(true);
    let handle = first.persistence.as_ref().unwrap().clone();
    let id = handle.id();
    let saved = handle.store().fixture_load(&id).unwrap();
    assert_eq!(saved.value().get("model"), Some(&Value::Null));
    assert!(!saved.pending.active);
    assert_eq!(saved.input.draft.attachments[0].id, attachment.id);
    first.submit().unwrap();
    tests::finish(&mut first);
    first.persist_input(true);
    assert_eq!(first.state.editor.prompt(), draft);
    assert_eq!(
        handle
            .store()
            .fixture_load(&id)
            .unwrap()
            .value()
            .get("model"),
        Some(&Value::Null)
    );
    drop(handle);
    drop(first);

    let fixture = HttpFixture::new(vec![(200, completion("Configured reply", vec![]))]);
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let store = Store::new(directory.path().join("config"));
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent
        .enable_sessions(store.path().parent().unwrap())
        .unwrap();
    let mut resumed = if configured {
        store.save(&settings).unwrap();
        App::new(
            agent,
            SessionConfig {
                store,
                settings,
                bash: crate::tools::find_bash().unwrap(),
            },
            None,
        )
    } else {
        app(&directory)
    };
    resumed.resume_session(&id);
    assert_eq!(resumed.state.editor.prompt(), draft);
    if !configured {
        assert!(!resumed.agent.as_ref().unwrap().has_model());
        assert_eq!(
            resumed
                .persistence
                .as_ref()
                .unwrap()
                .snapshot()
                .value()
                .get("model"),
            Some(&Value::Null)
        );
        resumed.config.settings.api_key = "isolated-fixture-key".into();
        resumed
            .config
            .settings
            .credentials
            .insert("openrouter".into(), "isolated-fixture-key".into());
        resumed.catalog_services = Some(crate::backend::Services::fixture(&fixture.endpoint));
        resumed.apply_selection("fixture/model".into(), crate::effort::Effort::Default, true);
        assert!(
            !resumed
                .persistence
                .as_ref()
                .unwrap()
                .snapshot()
                .events
                .iter()
                .any(|event| event.get("type").and_then(Value::as_str) == Some("backend_changed"))
        );
    }
    assert_eq!(resumed.archive.model, "fixture/model");
    assert_eq!(resumed.state.editor.prompt(), draft);
    assert_eq!(resumed.state.editor.attachments[0].id, attachment.id);
    let stored = resumed
        .persistence
        .as_ref()
        .unwrap()
        .store()
        .attachments()
        .load(&attachment.id)
        .unwrap();
    assert_eq!(std::fs::read(stored.path).unwrap(), b"Keep this attachment");
    resumed.submit().unwrap();
    tests::finish(&mut resumed);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].body.get("model").and_then(Value::as_str),
        Some("fixture/model")
    );
}
