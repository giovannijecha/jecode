use super::*;
use crate::session::{End, history::Step};
use std::io::{Seek, SeekFrom};

#[test]
fn preclassification_v2_attempts_load_and_export_as_unknown_without_writing() {
    use crate::providers::openai_account::client::{Attempt, Delivery};
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, Some(&fixture.0), None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("legacy fixture".into()).unwrap();
    history.turns[0].steps.push(Step {
        attempts: vec![Attempt {
            delivery: Delivery::Streaming,
            ..Default::default()
        }],
        ..Default::default()
    });
    history.turns[0].end = Some(End::Failed(crate::session::Failure::Worker));
    history.checkpoint().unwrap();
    drop(history);

    // Reframe an isolated owned fixture in the old format, then update only
    // the fixture head's integrity fields. Real user sessions are untouched.
    let sessions = store.directory("sessions-v2").unwrap();
    let info = head::read(&sessions, &id).unwrap();
    let mut events = Vec::new();
    log::visit(
        &sessions,
        &id,
        info.committed,
        info.rolling,
        0,
        usize::MAX,
        |turn, event| {
            events.push((turn, event));
            Ok(())
        },
    )
    .unwrap();
    let mut removed = 0;
    for (_, event) in &mut events {
        if event.get("kind").and_then(Value::text) != Some("step") {
            continue;
        }
        let Value::Object(fields) = event else {
            unreachable!()
        };
        let Value::Object(step) = fields.get_mut("data").unwrap() else {
            unreachable!()
        };
        let Value::Array(attempts) = step.get_mut("attempts").unwrap() else {
            unreachable!()
        };
        for attempt in attempts {
            let Value::Object(fields) = attempt else {
                unreachable!()
            };
            removed += usize::from(fields.remove("provider_failure").is_some());
        }
    }
    assert!(removed > 0);
    let mut file = log::open(&sessions, &id, false).unwrap();
    file.set_len(0).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut committed = 0;
    let mut rolling = log::HASH_START;
    for (turn, event) in events {
        log::append(&mut file, turn, &event, &mut committed, &mut rolling).unwrap();
    }
    file.sync_all().unwrap();
    drop(file);
    let head_name = format!("{id}.head");
    let mut value = json::parse(
        &sessions
            .read(&head_name, head::HEAD_LIMIT)
            .unwrap()
            .unwrap(),
        Default::default(),
    )
    .unwrap();
    {
        let Value::Object(fields) = &mut value else {
            unreachable!()
        };
        fields.insert("committed".into(), Value::Number(committed.to_string()));
        fields.insert("rolling".into(), Value::Number(rolling.to_string()));
        fields.remove("integrity");
    }
    let integrity = log::fingerprint(&value).unwrap();
    let Value::Object(fields) = &mut value else {
        unreachable!()
    };
    fields.insert("integrity".into(), Value::Number(integrity.to_string()));
    sessions
        .replace(&head_name, &json::encode(&value, head::HEAD_LIMIT).unwrap())
        .unwrap();

    let log_name = format!("{id}.log");
    let before_head = sessions
        .read(&head_name, head::HEAD_LIMIT)
        .unwrap()
        .unwrap();
    let before_log = std::fs::read(sessions.root().join(&log_name)).unwrap();
    let directory = crate::session::scope::Directory::open(&fixture.0).unwrap();
    for _ in 0..2 {
        let attempts =
            super::super::recent_network_attempts_in_store(&store, &id, &directory).unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].attempt.provider_failure, None);
        assert_eq!(
            sessions
                .read(&head_name, head::HEAD_LIMIT)
                .unwrap()
                .unwrap(),
            before_head
        );
        assert_eq!(
            std::fs::read(sessions.root().join(&log_name)).unwrap(),
            before_log
        );
    }
    let saved = super::super::resume_in_store(&store, &id, &directory).unwrap();
    assert_eq!(
        saved.history.turns[0].steps[0].attempts[0].provider_failure,
        None
    );
}
