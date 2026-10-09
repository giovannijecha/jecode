use super::*;
use crate::test_support::{Directory, HttpFixture, completion};
use std::fs;

fn agent(home: &Directory, project: &Directory, endpoint: &str) -> Agent {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    for (role, content) in [("user", "Old request"), ("assistant", "Old answer")] {
        agent.messages.lock().unwrap().push(Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(content)),
        ]));
    }
    agent.save_session().unwrap();
    agent
}

#[test]
fn deleting_current_removes_owned_orphans_and_the_next_turn_saves_only_the_new_context() {
    let home = Directory::new();
    let project = Directory::new();
    let provider = HttpFixture::new(vec![(200, completion("New answer", vec![]))]);
    let mut current = agent(&home, &project, &provider.endpoint);
    fs::write(project.path().join("keep.txt"), "project data").unwrap();
    fs::write(project.path().join("manual-export.json"), "manual data").unwrap();
    let old = current.sessions().unwrap();
    let store = old.store();
    let id = old.id();
    let temporary = current.temporary_info().unwrap().path;
    fs::write(std::path::Path::new(&temporary).join("orphan.txt"), "probe").unwrap();
    let result = current.tools.execute(
        "bash",
        &Value::object([("command", Value::string("printf 'out'; printf 'err' >&2"))]).encode(),
    );
    assert!(result.get("error").is_none());
    let reference = result.get("stdout_file").unwrap().as_str().unwrap();
    assert!(reference.starts_with(&format!("output:{id}:")));
    // The output was never added to the journal; ownership still identifies it.
    assert!(store.output_directory().join(&id).exists());
    let (report, replaced) = current
        .delete_session(&id, "fixture/default".into(), Effort::High)
        .unwrap();
    assert!(replaced && report.files >= 6);
    assert!(!std::path::Path::new(&temporary).exists());
    assert!(!store.output_directory().join(&id).exists());
    assert!(store.list().unwrap().sessions.is_empty());
    assert_eq!(
        fs::read_to_string(project.path().join("keep.txt")).unwrap(),
        "project data"
    );
    assert_eq!(
        fs::read_to_string(project.path().join("manual-export.json")).unwrap(),
        "manual data"
    );
    let next_id = current.sessions().unwrap().id();
    current.run_turn("New request", &mut |_| Ok(())).unwrap();
    let requests = provider.finish();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].body.encode().contains("Old request"));
    assert!(requests[0].body.encode().contains("New request"));
    assert_eq!(current.effort(), Effort::High);
    let listing = store.list().unwrap();
    assert_eq!(listing.sessions.len(), 1);
    assert_eq!(listing.sessions[0].id, next_id);
    assert_eq!(listing.sessions[0].title, "New request");
    old.flush().unwrap();
    assert_eq!(store.list().unwrap().sessions.len(), 1);
}

#[test]
fn prepared_current_turn_cannot_be_deleted() {
    let home = Directory::new();
    let project = Directory::new();
    let mut current = agent(&home, &project, "http://127.0.0.1:1/chat/completions");
    let handle = current.sessions().unwrap();
    let id = handle.id();
    current.prepare_turn("Prepared request").unwrap();
    let before = current.messages.lock().unwrap().clone();
    assert!(
        current
            .delete_session(&id, "fixture/default".into(), Effort::Default)
            .unwrap_err()
            .contains("ready")
    );
    assert_eq!(current.sessions().unwrap().id(), id);
    assert_eq!(*current.messages.lock().unwrap(), before);
    assert!(handle.active());
    assert_eq!(handle.store().list().unwrap().sessions.len(), 1);
}

#[test]
fn deleting_a_session_releases_only_attachments_no_other_session_uses() {
    let home = Directory::new();
    let project = Directory::new();
    let mut current = agent(&home, &project, "http://127.0.0.1:1/chat/completions");
    let pool = current.tools.attachments().unwrap().clone();
    let shared = pool.import_bytes("shared.txt", b"shared").unwrap();
    let own = pool.import_bytes("own.bin", &[1, 2]).unwrap();
    let marker = crate::attachments::MARKER;
    let send = |agent: &Agent, attachments: Vec<crate::attachments::Attachment>| {
        let text = marker.to_string().repeat(attachments.len());
        let prompt = crate::attachments::Prompt::new(text, attachments);
        agent.messages.lock().unwrap().push(prompt.message());
        agent.save_session().unwrap();
    };
    send(&current, vec![shared.clone(), own.clone()]);
    let old = current.sessions().unwrap().id();
    current
        .start_new("fixture/model".into(), Effort::High)
        .unwrap();
    send(&current, vec![shared.clone()]);
    let (report, replaced) = current
        .delete_session(&old, "fixture/model".into(), Effort::High)
        .unwrap();
    assert!(!replaced && report.files >= 1);
    // Released ids skip the grace period that protects unsaved drafts.
    assert!(pool.load(&own.id).is_err());
    assert!(pool.load(&shared.id).is_ok());
}

#[test]
fn deleting_old_session_keeps_attachments_in_new_session_history() {
    let home = Directory::new();
    let project = Directory::new();
    let mut current = agent(&home, &project, "http://127.0.0.1:1/chat/completions");
    let pool = current.tools.attachments().unwrap().clone();
    let attachment = pool.import_bytes("history.txt", b"history").unwrap();
    let prompt = crate::attachments::Prompt::new(
        crate::attachments::MARKER.to_string(),
        vec![attachment.clone()],
    );
    current.messages.lock().unwrap().push(prompt.message());
    let previous = current.sessions().unwrap();
    previous.input(crate::sessions::Input {
        history: vec![prompt],
        ..Default::default()
    });
    current.save_session().unwrap();
    let old_id = previous.id();
    current
        .start_new("fixture/model".into(), Effort::High)
        .unwrap();
    let next = current.sessions().unwrap();
    assert_eq!(
        next.snapshot().input.history[0].attachments[0].id,
        attachment.id
    );
    assert!(
        next.store()
            .list()
            .unwrap()
            .sessions
            .iter()
            .any(|session| session.id == next.id())
    );
    drop(previous);
    current
        .delete_session(&old_id, "fixture/model".into(), Effort::High)
        .unwrap();
    assert!(pool.load(&attachment.id).is_ok());
    assert_eq!(next.store().list().unwrap().sessions.len(), 1);
}

#[test]
fn deleting_current_session_keeps_attachment_in_inherited_draft() {
    let home = Directory::new();
    let project = Directory::new();
    let mut current = agent(&home, &project, "http://127.0.0.1:1/chat/completions");
    let pool = current.tools.attachments().unwrap().clone();
    let attachment = pool.import_bytes("draft.txt", b"draft").unwrap();
    let prompt = crate::attachments::Prompt::new(
        crate::attachments::MARKER.to_string(),
        vec![attachment.clone()],
    );
    current.messages.lock().unwrap().push(prompt.message());
    let handle = current.sessions().unwrap();
    handle.input(crate::sessions::Input {
        draft: crate::sessions::Draft::from_prompt(prompt),
        ..Default::default()
    });
    current.save_session().unwrap();
    let id = handle.id();
    let (_, replaced) = current
        .delete_session(&id, "fixture/model".into(), Effort::High)
        .unwrap();
    assert!(replaced);
    assert!(pool.load(&attachment.id).is_ok());
    assert_eq!(
        current
            .sessions()
            .unwrap()
            .snapshot()
            .input
            .draft
            .attachments[0]
            .id,
        attachment.id
    );
}

#[test]
fn attachment_only_draft_is_saved_and_resumed_before_any_sent_request() {
    let home = Directory::new();
    let project = Directory::new();
    let mut current = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    current.enable_sessions(home.path()).unwrap();
    let handle = current.sessions().unwrap();
    let pool = handle.store().attachments();
    let attachment = pool.import_bytes("draft.txt", b"draft").unwrap();
    handle.input(crate::sessions::Input {
        draft: crate::sessions::Draft::from_prompt(crate::attachments::Prompt::new(
            crate::attachments::MARKER.to_string(),
            vec![attachment.clone()],
        )),
        ..Default::default()
    });
    handle.flush().unwrap();
    let id = handle.id();
    drop(handle);
    drop(current);
    let mut resumed = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(
        resumed
            .sessions()
            .unwrap()
            .snapshot()
            .input
            .draft
            .attachments[0]
            .id,
        attachment.id
    );
    assert!(pool.load(&attachment.id).is_ok());
}

#[test]
fn text_only_draft_still_does_not_create_a_session_before_first_request() {
    let home = Directory::new();
    let project = Directory::new();
    let mut current = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    current.enable_sessions(home.path()).unwrap();
    let handle = current.sessions().unwrap();
    handle.input(crate::sessions::Input {
        draft: crate::sessions::Draft::from_prompt("unsent text".into()),
        ..Default::default()
    });
    handle.flush().unwrap();
    assert!(handle.store().list().unwrap().sessions.is_empty());
}
