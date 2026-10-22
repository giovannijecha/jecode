//! Recovery lifecycle receipts and scoped command observation.
use super::*;
use crate::cancel::Cancellation;
use crate::checkpoints::policy::Policy;
use crate::checkpoints::record::Record;

#[test]
fn completion_observes_exit_after_recovery_was_accepted() {
    let fixture = Fixture::new();
    let workspace = fixture.open();
    let policy = Policy {
        recovery_accepted_hours: 0,
        ..Policy::default()
    };
    policy
        .save_locked(&workspace.store().workspace.directory)
        .unwrap();
    workspace
        .store()
        .transaction(true, |tx| {
            let seq = tx.seq();
            tx.record(Record::Policy {
                seq,
                id: policy.id(),
                text: policy.encode(),
            });
            Ok(())
        })
        .unwrap();

    let command = workspace.command_scoped_cancellable(
        "model",
        "background process",
        &["late.txt".into()],
        &Cancellation::default(),
    );
    let completion = command.completion().unwrap();
    let mut store = workspace.store();
    let group = crate::checkpoints::recovery::group_for(&store.index, "model").unwrap();
    store
        .transaction(true, |tx| {
            crate::checkpoints::recovery::accept(tx, group, "model")
        })
        .unwrap();
    drop(store);

    fixture.write("late.txt", b"process finished\n");
    completion.finish().unwrap();
    assert!(
        workspace.store().index.current("late.txt").is_some(),
        "the final process change must be observed even after acceptance"
    );
}

#[test]
fn failed_post_observation_has_no_complete_after_reference() {
    let fixture = Fixture::new();
    fixture.write("notes.txt", b"before\n");
    let workspace = fixture.open();
    let command = workspace.command_scoped_cancellable(
        "model",
        "write notes.txt",
        &["notes.txt".into()],
        &Cancellation::default(),
    );
    fixture.write("notes.txt", b"after\n");
    workspace.store().journal.fault = Some("synthetic postcapture failure".into());

    let report = command.finish();
    assert!(
        report.error.is_some() || !report.partial.is_empty(),
        "{report:?}"
    );
    assert!(
        report.after.is_none(),
        "an incomplete post-observation must not advertise a complete after reference: {report:?}"
    );
}

#[test]
fn accept_survives_retention_between_commit_and_response() {
    use crate::checkpoints::actions::{self, Caller};
    use crate::json::Value;

    let fixture = Fixture::new();
    fixture.write("notes.txt", b"before\n");
    let workspace = fixture.open();
    let policy = Policy {
        recovery_accepted_hours: 0,
        ..Policy::default()
    };
    policy
        .save_locked(&workspace.store().workspace.directory)
        .unwrap();
    workspace
        .store()
        .transaction(true, |tx| {
            let seq = tx.seq();
            tx.record(Record::Policy {
                seq,
                id: policy.id(),
                text: policy.encode(),
            });
            Ok(())
        })
        .unwrap();
    let _command = workspace.command_scoped_cancellable(
        "model",
        "work",
        &["notes.txt".into()],
        &Cancellation::default(),
    );

    let during_response = Arc::clone(&workspace);
    actions::test_after_accept(move || {
        std::thread::sleep(std::time::Duration::from_millis(2));
        during_response.collect(true).unwrap().unwrap();
        assert!(
            during_response.store().index.recovery.groups.is_empty(),
            "the accepted group should have expired before the response is assembled"
        );
    });
    let result = actions::run(
        &workspace,
        &Caller {
            origin: "model",
            user: false,
        },
        &Value::object([("action", Value::string("accept"))]),
    );
    assert!(
        result.is_ok(),
        "accept must return its committed receipt: {result:?}"
    );
    let receipt = result.unwrap();
    assert_eq!(
        receipt.get("status").and_then(Value::as_str),
        Some("accepted")
    );
    assert!(
        receipt
            .get("accepted_at")
            .and_then(Value::as_usize)
            .is_some()
    );
}

#[test]
fn user_accept_without_ref_selects_current_model_attempt() {
    use crate::checkpoints::actions::{self, Caller};
    use crate::json::Value;

    let fixture = Fixture::new();
    fixture.write("notes.txt", b"before\n");
    let workspace = fixture.open();
    let _model = workspace.command_scoped_cancellable(
        "conversation",
        "work",
        &["notes.txt".into()],
        &Cancellation::default(),
    );
    let model_group =
        crate::checkpoints::recovery::group_for(&workspace.store().index, "conversation").unwrap();
    let accepted = actions::run(
        &workspace,
        &Caller {
            origin: "user:conversation",
            user: true,
        },
        &Value::object([("action", Value::string("accept"))]),
    )
    .unwrap();
    let expected = crate::checkpoints::index::reference(model_group);
    assert_eq!(
        accepted.get("group").and_then(Value::as_str),
        Some(expected.as_str())
    );
    assert!(
        workspace.store().index.recovery.groups[&model_group]
            .accepted
            .is_some()
    );
}

#[test]
fn user_accept_prefers_model_attempt_over_manual_capture() {
    use crate::checkpoints::actions::{self, Caller};
    use crate::json::Value;

    let fixture = Fixture::new();
    fixture.write("model.txt", b"model before\n");
    fixture.write("manual.txt", b"manual before\n");
    let workspace = fixture.open();
    let _model = workspace.command_scoped_cancellable(
        "conversation",
        "work",
        &["model.txt".into()],
        &Cancellation::default(),
    );
    let caller = Caller {
        origin: "user:conversation",
        user: true,
    };
    let manual = actions::run(
        &workspace,
        &caller,
        &Value::object([
            ("action", Value::string("capture")),
            ("paths", Value::Array(vec![Value::string("manual.txt")])),
        ]),
    )
    .unwrap();
    assert_eq!(manual.get("status").and_then(Value::as_str), Some("saved"));
    let (model_group, manual_group) = {
        let store = workspace.store();
        (
            crate::checkpoints::recovery::group_for(&store.index, "conversation").unwrap(),
            crate::checkpoints::recovery::group_for(&store.index, "user:conversation").unwrap(),
        )
    };
    assert_ne!(model_group, manual_group);

    let accepted = actions::run(
        &workspace,
        &caller,
        &Value::object([("action", Value::string("accept"))]),
    )
    .unwrap();
    let expected = crate::checkpoints::index::reference(model_group);
    assert_eq!(
        accepted.get("group").and_then(Value::as_str),
        Some(expected.as_str())
    );
    assert!(
        workspace.store().index.recovery.groups[&manual_group]
            .accepted
            .is_none(),
        "a manual capture must require its explicit reference"
    );

    let explicit = actions::run(
        &workspace,
        &caller,
        &Value::object([
            ("action", Value::string("accept")),
            (
                "ref",
                Value::string(crate::checkpoints::index::reference(manual_group)),
            ),
        ]),
    )
    .unwrap();
    let expected = crate::checkpoints::index::reference(manual_group);
    assert_eq!(
        explicit.get("group").and_then(Value::as_str),
        Some(expected.as_str())
    );
}
