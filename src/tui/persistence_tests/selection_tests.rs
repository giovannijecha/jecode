use super::*;
use crate::{cli::Selection, permissions::Level};

fn saved(home: &Directory, directory: &Directory) -> String {
    let first = app(home, directory, "http://127.0.0.1:1/chat/completions");
    saved_exchange(&first);
    first.persistence.as_ref().unwrap().id()
}

#[test]
fn picker_resume_applies_launch_permissions_once() {
    let home = Directory::new();
    let directory = Directory::new();
    let id = saved(&home, &directory);
    let mut resumed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    let selection = Selection {
        permissions: Some(Level::Read),
        ..Selection::default()
    };
    resumed.prepare_resume(Some(None), &selection).unwrap();
    let index = resumed
        .saved_sessions
        .iter()
        .position(|saved| saved.id == id)
        .unwrap();
    resumed.choose(selector::Choice::Session(index)).unwrap();
    let agent = resumed.agent.as_ref().unwrap();
    assert_eq!(agent.permissions().level(), Level::Read);
    assert!(resumed.worker.is_none());

    resumed.dispatch("/permissions full".into()).unwrap();
    assert_eq!(
        resumed.agent.as_ref().unwrap().permissions().level(),
        Level::Full
    );
}

#[test]
fn explicit_resume_rejects_unavailable_launch_provider() {
    let home = Directory::new();
    let directory = Directory::new();
    let id = saved(&home, &directory);
    let mut resumed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    let selection = Selection {
        provider: Some("openai-api".into()),
        model: Some("fixture/model".into()),
        ..Selection::default()
    };
    assert!(resumed.prepare_resume(Some(Some(id)), &selection).is_err());
    assert!(resumed.worker.is_none());
}

#[test]
fn picker_resume_rejects_unavailable_launch_provider() {
    let home = Directory::new();
    let directory = Directory::new();
    let id = saved(&home, &directory);
    let mut resumed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    let selection = Selection {
        provider: Some("openai-api".into()),
        model: Some("fixture/model".into()),
        ..Selection::default()
    };
    resumed.prepare_resume(Some(None), &selection).unwrap();
    let index = resumed
        .saved_sessions
        .iter()
        .position(|saved| saved.id == id)
        .unwrap();
    assert!(resumed.choose(selector::Choice::Session(index)).is_err());
    assert!(resumed.worker.is_none());
}

#[test]
fn picker_keeps_launch_options_after_deleting_another_conversation() {
    let home = Directory::new();
    let directory = Directory::new();
    let removed = saved(&home, &directory);
    let retained = saved(&home, &directory);
    let mut resumed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    let selection = Selection {
        permissions: Some(Level::Read),
        ..Selection::default()
    };
    resumed.prepare_resume(Some(None), &selection).unwrap();
    delete_tests::confirm(&mut resumed, &removed);
    resumed.finish_delete(true).unwrap();
    let index = resumed
        .saved_sessions
        .iter()
        .position(|saved| saved.id == retained)
        .unwrap();
    resumed.choose(selector::Choice::Session(index)).unwrap();
    assert_eq!(
        resumed.agent.as_ref().unwrap().permissions().level(),
        Level::Read
    );
}

#[test]
fn cancelling_the_initial_picker_discards_its_launch_options() {
    let home = Directory::new();
    let directory = Directory::new();
    let id = saved(&home, &directory);
    let mut resumed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    let selection = Selection {
        permissions: Some(Level::Read),
        ..Selection::default()
    };
    resumed.prepare_resume(Some(None), &selection).unwrap();
    resumed
        .input(Decoded::Key(terminal::Key {
            code: 27,
            modifiers: 0,
            character: 0,
        }))
        .unwrap();
    assert!(resumed.state.selector.is_none());
    resumed.open_sessions();
    let index = resumed
        .saved_sessions
        .iter()
        .position(|saved| saved.id == id)
        .unwrap();
    resumed.choose(selector::Choice::Session(index)).unwrap();
    assert_eq!(
        resumed.agent.as_ref().unwrap().permissions().level(),
        Level::Workspace
    );
}
