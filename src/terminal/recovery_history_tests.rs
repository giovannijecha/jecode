//! Recovered input and the earlier draft have independent history traversals.
use super::*;

#[test]
fn history_next_after_retrieval_keeps_the_withdrawn_edit() {
    let (mut model, mut session, _, release) = running();
    enqueue(&mut model, &mut session, "queued message to recover");
    model.editor.insert("unsent original draft");
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    assert_eq!(model.editor.text, "original task");
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    assert_eq!(model.editor.text, "queued message to recover");
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "queued message to recover");
    release.send(()).unwrap();
    finish(&mut model, &mut session);
}

#[test]
fn recovered_browse_is_independent_and_submission_restores_the_old_traversal() {
    let (mut model, mut session, _, release) = running();
    enqueue(&mut model, &mut session, "recovered draft");
    model.editor.insert("unsent original\n  draft");
    model.editor.left();
    let original_cursor = model.editor.cursor;
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    assert_eq!(model.editor.text, "original task");
    let recalled_cursor = model.editor.cursor;

    account::input(&mut model, Key::RetrieveQueued, &mut session);
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "recovered draft");
    model.editor.left();
    let recovered_cursor = model.editor.cursor;
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    assert_eq!(model.editor.text, "original task");
    account::input(&mut model, Key::Text(" changed".into()), &mut session);
    assert_eq!(model.editor.text, "original task changed");
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "recovered draft");
    assert_eq!(model.editor.cursor, recovered_cursor);
    assert!(model.menu.pasted_literal);

    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(session.pending_guidance().snapshot(), ["recovered draft"]);
    assert_eq!(model.editor.text, "original task");
    assert_eq!(model.editor.cursor, recalled_cursor);
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "unsent original\n  draft");
    assert_eq!(model.editor.cursor, original_cursor);
    release.send(()).unwrap();
    finish(&mut model, &mut session);
}

#[test]
fn submitting_a_recalled_entry_cannot_discard_the_hidden_recovered_edit() {
    let (mut model, mut session, _, release) = running();
    enqueue(&mut model, &mut session, "recovered draft");
    model.editor.insert("prior draft");
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    account::input(&mut model, Key::Text(" revised".into()), &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.editor.text, "original task revised");
    assert!(model.account.as_ref().unwrap().recovery.is_some());
    assert!(session.pending_guidance().snapshot().is_empty());
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "recovered draft");
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.editor.text, "original task");
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "prior draft");
    release.send(()).unwrap();
    finish(&mut model, &mut session);
}

#[test]
fn multiline_slash_recovery_and_abandon_restore_both_navigation_states() {
    for abandon in [false, true] {
        let (mut model, mut session, _, release) = running();
        assert!(session.enqueue("/help\n  recovered step"));
        model.editor.insert("/literal\n  prior draft");
        model.editor.left();
        let original_cursor = model.editor.cursor;
        model.menu.pasted_literal = true;
        account::input(&mut model, Key::HistoryPrevious, &mut session);
        assert_eq!(model.editor.text, "original task");

        account::input(&mut model, Key::RetrieveQueued, &mut session);
        assert_eq!(model.editor.text, "/help\n  recovered step");
        model.editor.left();
        let recovered_cursor = model.editor.cursor;
        account::input(&mut model, Key::HistoryPrevious, &mut session);
        assert_eq!(model.editor.text, "original task");
        account::input(&mut model, Key::HistoryNext, &mut session);
        assert_eq!(model.editor.text, "/help\n  recovered step");
        assert_eq!(model.editor.cursor, recovered_cursor);
        assert!(model.menu.pasted_literal);

        account::input(
            &mut model,
            if abandon {
                Key::AbandonRecovered
            } else {
                Key::Enter
            },
            &mut session,
        );
        assert_eq!(model.editor.text, "original task");
        account::input(&mut model, Key::HistoryNext, &mut session);
        assert_eq!(model.editor.text, "/literal\n  prior draft");
        assert_eq!(model.editor.cursor, original_cursor);
        assert!(model.menu.pasted_literal);
        assert_eq!(
            session.pending_guidance().snapshot(),
            if abandon {
                Vec::<String>::new()
            } else {
                vec!["/help\n  recovered step".into()]
            },
        );
        release.send(()).unwrap();
        finish(&mut model, &mut session);
    }
}

#[test]
fn rejected_recovered_submission_preserves_the_prior_history_draft() {
    let (mut model, mut session, _, release) = running();
    enqueue(&mut model, &mut session, "recovered draft");
    model.editor.insert("prior draft");
    model.editor.left();
    let original_cursor = model.editor.cursor;
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    account::input(&mut model, Key::Escape, &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.editor.text, "recovered draft");
    assert!(model.account.as_ref().unwrap().recovery.is_some());
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "recovered draft");
    account::input(&mut model, Key::AbandonRecovered, &mut session);
    assert_eq!(model.editor.text, "original task");
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "prior draft");
    assert_eq!(model.editor.cursor, original_cursor);
    release.send(()).unwrap();
    finish(&mut model, &mut session);
}

#[test]
fn incoming_canonical_turn_remains_browsable_in_both_restored_paths() {
    for abandon in [false, true] {
        let (mut model, mut session, _, release) = running();
        enqueue(&mut model, &mut session, "recovered draft");
        model.editor.insert("prior draft");
        account::input(&mut model, Key::HistoryPrevious, &mut session);
        account::input(&mut model, Key::RetrieveQueued, &mut session);
        account::input(&mut model, Key::HistoryPrevious, &mut session);
        assert_eq!(model.editor.text, "original task");
        account::event(
            &mut model,
            Event::Guidance {
                text: "incoming canonical".into(),
                new_turn: true,
            },
        );
        account::input(&mut model, Key::HistoryNext, &mut session);
        assert_eq!(model.editor.text, "incoming canonical");
        account::input(&mut model, Key::HistoryNext, &mut session);
        assert_eq!(model.editor.text, "recovered draft");

        if abandon {
            account::input(&mut model, Key::AbandonRecovered, &mut session);
        } else {
            account::input(&mut model, Key::Enter, &mut session);
            assert_eq!(session.pending_guidance().snapshot(), ["recovered draft"]);
        }
        assert_eq!(model.editor.text, "original task");
        account::input(&mut model, Key::HistoryNext, &mut session);
        assert_eq!(model.editor.text, "incoming canonical");
        account::input(&mut model, Key::HistoryNext, &mut session);
        assert_eq!(model.editor.text, "prior draft");
        release.send(()).unwrap();
        finish(&mut model, &mut session);
    }
}

#[test]
fn history_eviction_during_recovery_keeps_drafts_and_bounds_navigation() {
    for abandon in [false, true] {
        let (mut model, mut session, _, release) = running();
        model
            .prompt_history
            .load((0..64).map(|index| format!("old {index}")).collect());
        enqueue(&mut model, &mut session, "recovered draft");
        model.editor.insert("prior draft");
        model.editor.left();
        let original_cursor = model.editor.cursor;
        for _ in 0..64 {
            account::input(&mut model, Key::HistoryPrevious, &mut session);
        }
        assert_eq!(model.editor.text, "old 0");
        account::input(&mut model, Key::RetrieveQueued, &mut session);
        for _ in 0..64 {
            account::input(&mut model, Key::HistoryPrevious, &mut session);
        }
        assert_eq!(model.editor.text, "old 0");
        for index in 64..66 {
            account::event(
                &mut model,
                Event::Guidance {
                    text: format!("new {index}"),
                    new_turn: true,
                },
            );
        }
        account::input(&mut model, Key::HistoryPrevious, &mut session);
        assert_eq!(model.editor.text, "old 0");
        account::input(&mut model, Key::HistoryNext, &mut session);
        assert_eq!(model.editor.text, "old 2");
        for _ in 0..64 {
            account::input(&mut model, Key::HistoryNext, &mut session);
        }
        assert_eq!(model.editor.text, "recovered draft");
        if abandon {
            account::input(&mut model, Key::AbandonRecovered, &mut session);
        } else {
            account::input(&mut model, Key::Enter, &mut session);
        }
        assert_eq!(model.editor.text, "old 0");
        account::input(&mut model, Key::HistoryPrevious, &mut session);
        assert_eq!(model.editor.text, "old 0");
        account::input(&mut model, Key::HistoryNext, &mut session);
        assert_eq!(model.editor.text, "old 2");
        for _ in 0..64 {
            account::input(&mut model, Key::HistoryNext, &mut session);
        }
        assert_eq!(model.editor.text, "prior draft");
        assert_eq!(model.editor.cursor, original_cursor);
        release.send(()).unwrap();
        finish(&mut model, &mut session);
    }
}
