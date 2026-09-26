use super::*;
use crate::{
    providers::openai_account::{Progress, Request, Response, Status, client},
    session::{self, End, Event, Session, TestBackend},
    tls::Budget,
};
use std::{ops::ControlFlow, sync::mpsc, time::Duration};

#[path = "recovery_history_tests.rs"]
mod history_tests;

struct Gated {
    entered: mpsc::SyncSender<usize>,
    release: mpsc::Receiver<()>,
    requests: usize,
    incomplete_first: bool,
}

impl TestBackend for Gated {
    fn login(
        &mut self,
        _: &Budget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }

    fn generate(
        &mut self,
        _: &Request,
        budget: &Budget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.requests += 1;
        let _ = self.entered.send(self.requests);
        if self.requests == 1 {
            let _ = self.release.recv_timeout(Duration::from_secs(5));
        }
        budget.check()?;
        Ok(session::tests::response(
            "done",
            if self.requests == 1 && self.incomplete_first {
                Status::Incomplete
            } else {
                Status::Completed
            },
        ))
    }
}

fn running(
    incomplete_first: bool,
) -> (
    model::Model,
    Session,
    mpsc::Receiver<usize>,
    mpsc::SyncSender<()>,
) {
    let (entered_tx, entered_rx) = mpsc::sync_channel(8);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let mut session = Session::with_backend(
        session::Model::Luna,
        Gated {
            entered: entered_tx,
            release: release_rx,
            requests: 0,
            incomplete_first,
        },
        None,
    )
    .unwrap();
    let mut model = account::model(session::Model::Luna, None);
    account::event(&mut model, session::tests::next(&mut session));
    account::input(&mut model, Key::Text("original task".into()), &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(entered_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
    (model, session, entered_rx, release_tx)
}

fn queue(model: &mut model::Model, session: &mut Session, text: &str) {
    account::input(model, Key::Text(text.into()), session);
    account::input(model, Key::Enter, session);
}

fn finish(model: &mut model::Model, session: &mut Session) -> End {
    loop {
        let event = session::tests::next(session);
        let outcome = match event {
            Event::Finished(end, _) => Some(end),
            _ => None,
        };
        account::event(model, event);
        if let Some(end) = outcome {
            account::after_finished(model, session, end);
            return end;
        }
    }
}

#[test]
fn pending_messages_are_claimed_fifo_as_distinct_turns() {
    let (mut model, mut session, entered, release) = running(false);
    queue(&mut model, &mut session, "second task");
    queue(&mut model, &mut session, "third task");
    assert_eq!(
        model.account.as_ref().unwrap().pending_messages(),
        ["second task", "third task"]
    );
    assert_eq!(
        model
            .blocks
            .iter()
            .filter(|block| block.speaker == "You")
            .count(),
        1
    );
    release.send(()).unwrap();
    assert_eq!(finish(&mut model, &mut session), End::Complete);
    assert_eq!(entered.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    assert_eq!(finish(&mut model, &mut session), End::Complete);
    assert_eq!(entered.recv_timeout(Duration::from_secs(5)).unwrap(), 3);
    assert_eq!(finish(&mut model, &mut session), End::Complete);
    let prompts: Vec<_> = model
        .blocks
        .iter()
        .filter(|block| block.speaker == "You")
        .map(|block| block.text.as_str())
        .collect();
    assert_eq!(prompts, ["original task", "second task", "third task"]);
    assert!(
        model
            .account
            .as_ref()
            .unwrap()
            .pending_messages()
            .is_empty()
    );
    assert!(session.pending_guidance().snapshot().is_empty());
}

#[test]
fn stop_restores_every_unsent_message_then_the_existing_draft() {
    let (mut model, mut session, entered, release) = running(false);
    queue(&mut model, &mut session, "first pending");
    queue(&mut model, &mut session, "second pending");
    account::input(&mut model, Key::Text("current draft".into()), &mut session);
    account::input(&mut model, Key::Escape, &mut session);
    assert_eq!(
        model.editor.text,
        "first pending\nsecond pending\ncurrent draft"
    );
    assert!(
        model
            .account
            .as_ref()
            .unwrap()
            .pending_messages()
            .is_empty()
    );
    release.send(()).unwrap();
    assert!(matches!(
        finish(&mut model, &mut session),
        End::Failed(_) | End::Incomplete
    ));
    assert!(entered.recv_timeout(Duration::from_millis(100)).is_err());
    assert_eq!(
        model
            .blocks
            .iter()
            .filter(|block| block.speaker == "You")
            .count(),
        1
    );
}

#[test]
fn incomplete_turn_does_not_automatically_resend_pending_work() {
    let (mut model, mut session, entered, release) = running(true);
    queue(&mut model, &mut session, "do not retry");
    release.send(()).unwrap();
    assert_eq!(finish(&mut model, &mut session), End::Incomplete);
    assert_eq!(model.editor.text, "do not retry");
    assert!(entered.recv_timeout(Duration::from_millis(100)).is_err());
    assert!(session.pending_guidance().snapshot().is_empty());
}

#[test]
fn withdrawing_the_latest_pending_message_keeps_the_hidden_draft() {
    let (mut model, mut session, entered, release) = running(false);
    queue(&mut model, &mut session, "first pending");
    queue(&mut model, &mut session, "latest pending");
    account::input(&mut model, Key::Text("earlier draft".into()), &mut session);
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    assert_eq!(model.editor.text, "latest pending");
    assert_eq!(
        model.account.as_ref().unwrap().pending_messages(),
        ["first pending"]
    );
    account::input(&mut model, Key::Text(" revised".into()), &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.editor.text, "earlier draft");
    assert_eq!(
        model.account.as_ref().unwrap().pending_messages(),
        ["first pending", "latest pending revised"]
    );
    release.send(()).unwrap();
    finish(&mut model, &mut session);
    assert_eq!(
        model.account.as_ref().unwrap().pending_messages(),
        ["latest pending revised"]
    );
    assert_eq!(entered.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    finish(&mut model, &mut session);
    assert!(
        model
            .account
            .as_ref()
            .unwrap()
            .pending_messages()
            .is_empty()
    );
    assert_eq!(entered.recv_timeout(Duration::from_secs(5)).unwrap(), 3);
    finish(&mut model, &mut session);
}

#[test]
fn whole_queue_recovery_is_lossless_beyond_one_prompt_limit() {
    let (mut model, mut session, _, release) = running(false);
    for index in 0..8 {
        queue(
            &mut model,
            &mut session,
            &format!("{index}{}", "x".repeat(40_000)),
        );
    }
    assert!(model.account.as_ref().unwrap().pending_messages().len() == 8);
    queue(&mut model, &mut session, "ninth draft");
    assert_eq!(model.editor.text, "ninth draft");
    account::input(&mut model, Key::Interrupt, &mut session);
    assert!(model.editor.text.len() > session::MAX_PROMPT_BYTES);
    for index in 0..8 {
        assert!(
            model
                .editor
                .text
                .contains(&format!("{index}{}", "x".repeat(40_000)))
        );
    }
    assert!(model.editor.text.ends_with("ninth draft"));
    release.send(()).unwrap();
    finish(&mut model, &mut session);
}
