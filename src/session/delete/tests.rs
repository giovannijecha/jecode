use super::*;
use crate::{
    config::{Settings, Store},
    json::Value,
    openrouter::OpenRouter,
    test_support::Directory,
    tools::Tools,
};
use std::io::Cursor;

fn fixture() -> (Directory, Agent, SessionConfig) {
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent
        .enable_sessions(&directory.path().join("home"))
        .unwrap();
    let config = SessionConfig {
        store: Store::new(directory.path().join("home")),
        settings: Settings::new("isolated-key".into(), "fixture/default".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    (directory, agent, config)
}
fn seed(agent: &Agent) {
    for (role, content) in [("user", "Sent request"), ("assistant", "Saved response")] {
        agent
            .archive()
            .messages
            .lock()
            .unwrap()
            .push(Value::object([
                ("role", Value::string(role)),
                ("content", Value::string(content)),
            ]));
    }
    agent.save_session().unwrap();
}

#[test]
fn default_cancel_and_eof_never_delete_plain_sessions() {
    let (_directory, mut agent, config) = fixture();
    seed(&agent);
    let id = agent.sessions().unwrap().id();
    let (sessions, _) = sessions(&agent).unwrap();
    let current = sessions.iter().find(|entry| entry.id == id).unwrap();
    for answer in ["", "\n", "/cancel\n", "yes\n"] {
        assert_eq!(
            confirm(
                &mut agent,
                &config,
                current,
                &mut Cursor::new(answer),
                &mut vec![],
            )
            .unwrap(),
            None
        );
        assert_eq!(
            agent
                .sessions()
                .unwrap()
                .store()
                .list()
                .unwrap()
                .sessions
                .len(),
            1
        );
        assert_eq!(agent.sessions().unwrap().id(), id);
    }
}

#[test]
fn plain_numbered_current_delete_resets_context_without_saving_an_empty_replacement() {
    let (_directory, mut agent, config) = fixture();
    seed(&agent);
    let old = agent.sessions().unwrap();
    let id = old.id();
    let (sessions, _) = sessions(&agent).unwrap();
    let current = sessions.iter().find(|entry| entry.id == id).unwrap();
    let mut output = vec![];
    assert_eq!(
        confirm(
            &mut agent,
            &config,
            current,
            &mut Cursor::new("delete\n"),
            &mut output,
        )
        .unwrap(),
        Some(true)
    );
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("current") && output.contains("new conversation opened"));
    assert_ne!(agent.sessions().unwrap().id(), id);
    assert_eq!(agent.archive().messages.lock().unwrap().len(), 1);
    agent.save_session().unwrap();
    old.flush().unwrap();
    assert!(
        agent
            .sessions()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .is_empty()
    );
}

#[test]
fn unsent_current_is_absent_from_the_list() {
    let (_directory, agent, _) = fixture();
    let (entries, warnings) = sessions(&agent).unwrap();
    assert!(warnings.is_empty());
    assert!(entries.is_empty());
}
