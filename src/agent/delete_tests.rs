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
