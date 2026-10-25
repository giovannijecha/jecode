use super::*;
use crate::{
    backend::{Services, unconfigured},
    config::{Settings, Store},
    effort::Effort,
    json::Value,
    test_support::Directory,
    tools::Tools,
};

fn app(directory: &Directory, configured: bool, selected: bool) -> App {
    let store = Store::new(directory.path().join("config"));
    let mut settings = if selected {
        Settings::selected("openai", String::new(), "saved-model".into()).unwrap()
    } else {
        Settings::unconfigured()
    };
    settings.effort = Effort::High;
    if selected {
        store.save(&settings).unwrap();
    }
    let client = if configured {
        let mut client = Services::fixture("http://127.0.0.1:1/chat/completions")
            .create("fixture/fallback".into())
            .unwrap();
        client.set_effort(Effort::Low);
        client
    } else {
        unconfigured::client(
            &settings.backend_id,
            selected.then(|| settings.model.clone()),
            None,
            "No configured provider".into(),
        )
    };
    let mut agent = Agent::with_backend(client, Tools::new(directory.path()).unwrap());
    agent
        .enable_sessions(store.path().parent().unwrap())
        .unwrap();
    App::new(
        agent,
        SessionConfig {
            store,
            settings,
            bash: crate::tools::find_bash().unwrap(),
        },
        None,
    )
}

fn seed(app: &mut App) -> String {
    app.archive.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Keep the old conversation")),
    ]));
    app.agent.as_ref().unwrap().save_session().unwrap();
    app.persistence.as_ref().unwrap().id()
}

fn assert_fallback(app: &App, old: &str) {
    let agent = app.agent.as_ref().unwrap();
    assert_ne!(app.persistence.as_ref().unwrap().id(), old);
    assert_eq!(agent.backend_id(), "openrouter");
    assert_eq!(agent.model(), "fixture/fallback");
    assert_eq!(agent.effort(), Effort::Low);
    assert!(!agent.needs_configuration());
    assert_eq!(app.archive.messages.lock().unwrap().len(), 1);
    assert_eq!(app.state.editor.text, "kept draft");
    assert_eq!(app.state.queue.messages()[0].text, "kept follow-up");
    assert_eq!(app.config.settings.backend_id, "openai");
    assert_eq!(app.config.settings.model, "saved-model");
    assert!(!matches!(
        app.state.notice.as_ref().unwrap().kind,
        Kind::Error
    ));
}

#[test]
fn new_in_fallback_keeps_the_active_selection_and_unsent_input() {
    let directory = Directory::new();
    let mut app = app(&directory, true, true);
    let saved = std::fs::read(app.config.store.path()).unwrap();
    let old = seed(&mut app);
    app.state.editor.insert("kept draft");
    app.state.queue.push("kept follow-up").unwrap();
    app.dispatch("/new".into()).unwrap();
    assert_fallback(&app, &old);
    assert!(
        app.state
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("current model kept")
    );
    assert_eq!(std::fs::read(app.config.store.path()).unwrap(), saved);
    assert!(
        app.persistence
            .as_ref()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .iter()
            .any(|session| session.id == old)
    );
}

#[test]
fn deleting_current_in_fallback_keeps_the_active_selection_and_unsent_input() {
    let directory = Directory::new();
    let mut app = app(&directory, true, true);
    let saved = std::fs::read(app.config.store.path()).unwrap();
    let old = seed(&mut app);
    app.state.editor.insert("kept draft");
    app.state.queue.push("kept follow-up").unwrap();
    app.open_sessions();
    app.start_delete(&old).unwrap();
    app.finish_delete(true).unwrap();
    assert_fallback(&app, &old);
    assert_eq!(std::fs::read(app.config.store.path()).unwrap(), saved);
    assert!(app.saved_sessions.is_empty());
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
}

#[test]
fn new_without_a_usable_selection_keeps_the_draft_and_offers_configuration() {
    for selected in [true, false] {
        let directory = Directory::new();
        let mut app = app(&directory, false, selected);
        let old = app.persistence.as_ref().unwrap().id();
        app.state.editor.insert("kept draft");
        app.dispatch("/new".into()).unwrap();
        let notice = app.state.notice.as_ref().unwrap();
        assert!(matches!(notice.kind, Kind::Warning));
        assert_eq!(
            notice.action.unwrap().0,
            if selected { "/settings" } else { "/model" }
        );
        assert_eq!(app.persistence.as_ref().unwrap().id(), old);
        assert_eq!(app.state.editor.text, "kept draft");
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
    }
}

#[test]
fn deleting_current_without_a_usable_selection_warns_before_starting_the_job() {
    for selected in [true, false] {
        let directory = Directory::new();
        let mut app = app(&directory, false, selected);
        let old = app.persistence.as_ref().unwrap().id();
        app.state.editor.insert("kept draft");
        app.start_delete(&old).unwrap();
        assert!(app.deletion_job.is_none());
        assert!(app.agent.is_some());
        let notice = app.state.notice.as_ref().unwrap();
        assert!(matches!(notice.kind, Kind::Warning));
        assert_eq!(
            notice.action.unwrap().0,
            if selected { "/settings" } else { "/model" }
        );
        assert_eq!(app.persistence.as_ref().unwrap().id(), old);
        assert_eq!(app.state.editor.text, "kept draft");
    }
}

#[test]
fn deleting_another_session_needs_no_provider_or_model() {
    let directory = Directory::new();
    let mut old = app(&directory, true, true);
    let id = seed(&mut old);
    drop(old);
    let mut app = app(&directory, false, false);
    let current = app.persistence.as_ref().unwrap().id();
    app.open_sessions();
    app.start_delete(&id).unwrap();
    app.finish_delete(true).unwrap();
    assert_eq!(app.persistence.as_ref().unwrap().id(), current);
    assert!(app.saved_sessions.is_empty());
    assert!(!app.agent.as_ref().unwrap().has_model());
}

#[test]
fn a_connected_alternative_does_not_leave_a_placeholder_default_available() {
    let directory = Directory::new();
    let mut app = app(&directory, false, true);
    let mut client = Services::fixture("http://127.0.0.1:1/chat/completions")
        .create("fixture/fallback".into())
        .unwrap();
    client.set_effort(Effort::Low);
    app.agent.as_mut().unwrap().replace_backend(client).unwrap();
    app.archive = app.agent.as_ref().unwrap().archive();
    let old = seed(&mut app);
    app.state.editor.insert("kept draft");
    app.state.queue.push("kept follow-up").unwrap();
    app.dispatch("/new".into()).unwrap();
    assert_fallback(&app, &old);
}

#[test]
fn new_and_current_deletion_use_the_default_when_its_account_is_available() {
    for delete in [false, true] {
        let directory = Directory::new();
        let mut app = app(&directory, true, true);
        let home = app.config.store.path().parent().unwrap().to_path_buf();
        crate::auth::openai::Store::fixture_account(home.clone(), "isolated-access-token").unwrap();
        let services = app.config.settings.services(&home, "openai").unwrap();
        app.agent.as_mut().unwrap().register_backend(services);
        let old = seed(&mut app);
        if delete {
            app.start_delete(&old).unwrap();
            app.finish_delete(true).unwrap();
        } else {
            app.dispatch("/new".into()).unwrap();
            assert!(
                app.state
                    .notice
                    .as_ref()
                    .unwrap()
                    .text
                    .contains("saved defaults applied")
            );
        }
        let agent = app.agent.as_ref().unwrap();
        assert_ne!(app.persistence.as_ref().unwrap().id(), old);
        assert_eq!(agent.backend_id(), "openai");
        assert_eq!(agent.model(), "saved-model");
        assert_eq!(agent.effort(), Effort::High);
    }
}

#[test]
fn a_newly_connected_default_can_start_a_conversation_before_model_selection() {
    let directory = Directory::new();
    let mut app = app(&directory, false, true);
    let home = app.config.store.path().parent().unwrap().to_path_buf();
    crate::auth::openai::Store::fixture_account(home.clone(), "isolated-access-token").unwrap();
    let services = app.config.settings.services(&home, "openai").unwrap();
    app.agent.as_mut().unwrap().register_backend(services);
    let old = app.persistence.as_ref().unwrap().id();
    app.dispatch("/new".into()).unwrap();
    assert_ne!(app.persistence.as_ref().unwrap().id(), old);
    assert!(!app.agent.as_ref().unwrap().needs_configuration());
    assert_eq!(app.archive.model, "saved-model");
}

#[test]
fn new_and_current_deletion_also_work_with_chatgpt_as_the_fallback() {
    for delete in [false, true] {
        let directory = Directory::new();
        let mut app = app(&directory, false, false);
        let home = app.config.store.path().parent().unwrap().to_path_buf();
        crate::auth::openai::Store::fixture_account(home.clone(), "isolated-access-token").unwrap();
        app.config.settings =
            Settings::selected("openrouter", String::new(), "missing-model".into()).unwrap();
        app.config.store.save(&app.config.settings).unwrap();
        let saved = std::fs::read(app.config.store.path()).unwrap();
        let services = app.config.settings.services(&home, "openai").unwrap();
        let mut client = services.create("chatgpt-fallback".into()).unwrap();
        client.set_effort(Effort::Low);
        app.agent.as_mut().unwrap().replace_backend(client).unwrap();
        app.archive = app.agent.as_ref().unwrap().archive();
        let old = seed(&mut app);
        if delete {
            app.start_delete(&old).unwrap();
            app.finish_delete(true).unwrap();
        } else {
            app.dispatch("/new".into()).unwrap();
        }
        let agent = app.agent.as_ref().unwrap();
        assert_ne!(app.persistence.as_ref().unwrap().id(), old);
        assert_eq!(agent.backend_id(), "openai");
        assert_eq!(agent.model(), "chatgpt-fallback");
        assert_eq!(agent.effort(), Effort::Low);
        assert_eq!(std::fs::read(app.config.store.path()).unwrap(), saved);
    }
}
