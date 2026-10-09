use super::super::*;
use crate::{
    config::{Settings, Store},
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion},
    tools::Tools,
};
use std::io::Cursor;

fn config(home: &Directory) -> SessionConfig {
    SessionConfig {
        store: Store::new(home.path().to_path_buf()),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    }
}

#[test]
fn plain_chat_stages_files_until_the_next_message() {
    let home = Directory::new();
    let project = Directory::new();
    std::fs::write(project.path().join("notes.txt"), "staged text").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("Read.", vec![])),
        (200, completion("Plain.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    let mut config = SessionConfig {
        store: Store::new(home.path().join("config.json")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    };
    let notes = project.path().join("notes.txt");
    let mut input = Cursor::new(format!(
        "/attach missing.txt\n/attach \"{}\"\n\nthen plain\n/exit\n",
        notes.display()
    ));
    let mut output = Vec::new();
    let mut status = Vec::new();
    chat(
        &mut agent,
        &mut config,
        &mut input,
        &mut output,
        &mut status,
        Vec::new(),
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(String::from_utf8(status).unwrap().contains("Cannot attach"));
    assert!(output.contains("Staged [1# File: notes.txt] for your next message."));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    let last = |index: usize| {
        requests[index]
            .body
            .get("messages")
            .and_then(Value::as_array)
            .unwrap()
            .last()
            .unwrap()
            .encode()
    };
    // Enter alone sends only the staged attachment; the next line is plain.
    assert!(last(0).contains("attachment:att-"));
    assert!(!last(1).contains("attachment:att-"));
}

#[test]
fn staged_file_survives_exit_source_removal_and_plain_resume() {
    let home = Directory::new();
    let project = Directory::new();
    let source = project.path().join("payload.txt");
    std::fs::write(&source, b"retained bytes").unwrap();
    let mut first = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    chat(
        &mut first,
        &mut config(&home),
        &mut Cursor::new(format!("/attach \"{}\"\n/exit\n", source.display())),
        &mut Vec::new(),
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let handle = first.sessions().unwrap();
    let id = handle.id();
    let attachment = handle.snapshot().input.staged[0].clone();
    let pool = handle.store().attachments();
    assert!(
        handle
            .store()
            .list()
            .unwrap()
            .sessions
            .iter()
            .any(|entry| entry.id == id)
    );
    drop(handle);
    drop(first);
    std::fs::remove_file(&source).unwrap();
    assert_eq!(
        std::fs::read(pool.load(&attachment.id).unwrap().path).unwrap(),
        b"retained bytes"
    );

    let fixture = HttpFixture::new(vec![(200, completion("Resumed.", vec![]))]);
    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    let mut output = Vec::new();
    chat_with_resume(
        &mut resumed,
        &mut config(&home),
        &mut Cursor::new("\n/exit\n"),
        &mut output,
        &mut Vec::new(),
        Some(id),
    )
    .unwrap();
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Staged [1# File: payload.txt]")
    );
    assert!(
        resumed
            .sessions()
            .unwrap()
            .snapshot()
            .input
            .staged
            .is_empty()
    );
    assert!(
        fixture.finish()[0]
            .body
            .encode()
            .contains(&attachment.reference())
    );
}

#[test]
fn new_conversation_keeps_plain_staging_for_its_next_message() {
    let home = Directory::new();
    let project = Directory::new();
    let source = project.path().join("for-next.txt");
    std::fs::write(&source, b"for next").unwrap();
    let fixture = HttpFixture::new(vec![(200, completion("Next.", vec![]))]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    chat(
        &mut agent,
        &mut config(&home),
        &mut Cursor::new(format!("/attach \"{}\"\n/new\n\n/exit\n", source.display())),
        &mut Vec::new(),
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert!(agent.sessions().unwrap().snapshot().input.staged.is_empty());
    assert!(
        fixture.finish()[0]
            .body
            .encode()
            .contains("attachment:att-")
    );
}

#[test]
fn plain_staging_can_be_cleared_without_sending() {
    let home = Directory::new();
    let project = Directory::new();
    let source = project.path().join("discard.txt");
    std::fs::write(&source, b"discard me").unwrap();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    let mut output = Vec::new();
    chat(
        &mut agent,
        &mut config(&home),
        &mut Cursor::new(format!(
            "/attach \"{}\"\n/attach --clear\n/exit\n",
            source.display()
        )),
        &mut output,
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert!(agent.sessions().unwrap().snapshot().input.staged.is_empty());
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Staged attachments cleared.")
    );
}

#[test]
fn resuming_another_session_transfers_plain_staging() {
    let home = Directory::new();
    let project = Directory::new();
    let seeded = HttpFixture::new(vec![(200, completion("Seeded.", vec![]))]);
    let mut earlier = Agent::new(
        OpenRouter::fixture(seeded.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    earlier.enable_sessions(home.path()).unwrap();
    earlier.run_turn("earlier", &mut |_| Ok(())).unwrap();
    let resume_id = earlier.sessions().unwrap().id();
    assert!(
        earlier
            .sessions()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .iter()
            .any(|entry| entry.id == resume_id)
    );
    drop(earlier);
    seeded.finish();

    let source = project.path().join("transfer.txt");
    std::fs::write(&source, b"transfer bytes").unwrap();
    let fixture = HttpFixture::new(vec![(200, completion("Transferred.", vec![]))]);
    let mut current = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    let mut output = Vec::new();
    let mut status = Vec::new();
    chat(
        &mut current,
        &mut config(&home),
        &mut Cursor::new(format!(
            "/attach \"{}\"\n/resume {resume_id}\n\n/exit\n",
            source.display()
        )),
        &mut output,
        &mut status,
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        current.sessions().unwrap().id(),
        resume_id,
        "{}\n{}",
        String::from_utf8_lossy(&output),
        String::from_utf8_lossy(&status)
    );
    assert!(
        current
            .sessions()
            .unwrap()
            .snapshot()
            .input
            .staged
            .is_empty()
    );
    assert!(
        fixture.finish()[0]
            .body
            .encode()
            .contains("attachment:att-")
    );
}

#[test]
fn deleting_current_plain_session_keeps_staged_reference_until_send() {
    let home = Directory::new();
    let project = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("First.", vec![])),
        (200, completion("Second.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let pool = agent.sessions().unwrap().store().attachments();
    let attachment = pool.import_bytes("owned.txt", b"owned bytes").unwrap();
    let prompt = crate::attachments::Prompt::new(
        crate::attachments::MARKER.to_string(),
        vec![attachment.clone()],
    );
    agent.run_turn(prompt, &mut |_| Ok(())).unwrap();
    let old_id = agent.sessions().unwrap().id();
    let input = format!(
        "/attach {}\n/resume\nd {old_id}\ndelete\n/cancel\n\n/exit\n",
        attachment.reference()
    );
    chat(
        &mut agent,
        &mut config(&home),
        &mut Cursor::new(input),
        &mut Vec::new(),
        &mut Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert_ne!(agent.sessions().unwrap().id(), old_id);
    assert!(pool.load(&attachment.id).is_ok());
    assert!(agent.sessions().unwrap().snapshot().input.staged.is_empty());
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].body.encode().contains(&attachment.reference()));
}

#[test]
fn deleting_another_session_keeps_a_plain_staged_reference() {
    let home = Directory::new();
    let project = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("First.", vec![]))]);
    let mut owner = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    owner.enable_sessions(home.path()).unwrap();
    let pool = owner.sessions().unwrap().store().attachments();
    let attachment = pool.import_bytes("shared.txt", b"shared bytes").unwrap();
    owner
        .run_turn(
            crate::attachments::Prompt::new(
                crate::attachments::MARKER.to_string(),
                vec![attachment.clone()],
            ),
            &mut |_| Ok(()),
        )
        .unwrap();
    let old_id = owner.sessions().unwrap().id();
    drop(owner);
    fixture.finish();

    let mut other = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    other.enable_sessions(home.path()).unwrap();
    let mut staged = Vec::new();
    super::stage(
        &other,
        &attachment.reference(),
        &mut staged,
        &mut Vec::new(),
    )
    .unwrap();
    let current_id = other.sessions().unwrap().id();
    assert!(
        other
            .sessions()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .iter()
            .any(|entry| entry.id == current_id)
    );
    other
        .delete_session(
            &old_id,
            "fixture/model".into(),
            crate::effort::Effort::Default,
        )
        .unwrap();
    assert_eq!(staged, vec![attachment.clone()]);
    assert!(pool.load(&attachment.id).is_ok());
}

#[test]
fn failed_prepare_keeps_staged_attachments_for_retry() {
    let home = Directory::new();
    let project = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let attachment = agent
        .sessions()
        .unwrap()
        .store()
        .attachments()
        .import_bytes("pending.txt", b"pending bytes")
        .unwrap();
    agent.prepare_turn("different prepared prompt").unwrap();
    chat(
        &mut agent,
        &mut config(&home),
        &mut Cursor::new("\n/exit\n"),
        &mut Vec::new(),
        &mut Vec::new(),
        vec![attachment.clone()],
    )
    .unwrap();
    assert_eq!(
        agent.sessions().unwrap().snapshot().input.staged,
        vec![attachment]
    );
}
