use super::*;
use crate::{
    providers::openai_account::{Progress, Request, Response, Status, client},
    session::{self, Event, Session, TestBackend},
    tls::Budget,
};
use std::{ops::ControlFlow, sync::mpsc, time::Duration};

struct Gated {
    entered: mpsc::SyncSender<()>,
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
        if self.requests == 1 {
            self.entered.send(()).unwrap();
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
fn running() -> (
    model::Model,
    Session,
    mpsc::Receiver<()>,
    mpsc::SyncSender<()>,
) {
    running_with_failure(false)
}
fn running_with_failure(
    incomplete_first: bool,
) -> (
    model::Model,
    Session,
    mpsc::Receiver<()>,
    mpsc::SyncSender<()>,
) {
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
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
    account::attach_queue(&mut model, &session);
    account::input(&mut model, Key::Text("original task".into()), &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    (model, session, entered_rx, release_tx)
}
fn enqueue(model: &mut model::Model, session: &mut Session, text: &str) {
    account::input(model, Key::Text(text.into()), session);
    account::input(model, Key::Enter, session);
}
fn finish(model: &mut model::Model, session: &mut Session) {
    loop {
        let event = session::tests::next(session);
        let finished = matches!(event, Event::Finished(..));
        account::event(model, event);
        if finished {
            break;
        }
    }
}

#[test]
fn recovered_edit_restores_multiline_draft_cursor_and_normal_queue_order() {
    let (mut model, mut session, _, release) = running();
    enqueue(&mut model, &mut session, "first guidance");
    enqueue(&mut model, &mut session, "latest guidance");
    model.editor.insert("/draft\n  keep 👩‍💻");
    model.editor.left();
    let cursor = model.editor.cursor;
    model.menu.pasted_literal = true;
    model.menu.hidden = true;
    let before = model.blocks.len();
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    assert_eq!(model.editor.text, "latest guidance");
    assert_eq!(session.pending_guidance().snapshot(), ["first guidance"]);
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    assert_eq!(model.editor.text, "latest guidance");
    account::input(&mut model, Key::Text(" edited".into()), &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.editor.text, "/draft\n  keep 👩‍💻");
    assert_eq!(model.editor.cursor, cursor);
    assert!(model.menu.pasted_literal && model.menu.hidden);
    assert!(model.account.as_ref().unwrap().recovery.is_none());
    assert_eq!(
        session.pending_guidance().snapshot(),
        ["first guidance", "latest guidance edited"]
    );
    assert_eq!(model.blocks.len(), before); // Unsent text never enters the transcript.
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    assert_eq!(model.editor.text, "original task");
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "/draft\n  keep 👩‍💻");
    assert_eq!(model.editor.cursor, cursor);
    release.send(()).unwrap();
    finish(&mut model, &mut session);
    assert!(session.pending_guidance().snapshot().is_empty());
}

#[test]
fn worker_can_finish_while_recovered_edit_waits_for_explicit_submission() {
    let (mut model, mut session, _, release) = running();
    enqueue(&mut model, &mut session, "recover me");
    model.editor.insert("prior\n  draft");
    model.editor.left();
    let cursor = model.editor.cursor;
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    release.send(()).unwrap();
    finish(&mut model, &mut session);
    assert_eq!(model.editor.text, "recover me");
    assert!(model.account.as_ref().unwrap().recovery.is_some());
    assert!(session.pending_guidance().snapshot().is_empty());
    assert_eq!(
        model
            .blocks
            .iter()
            .filter(|block| block.speaker == "You")
            .count(),
        1
    );
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.editor.text, "prior\n  draft");
    assert_eq!(model.editor.cursor, cursor);
    assert_eq!(
        model
            .blocks
            .iter()
            .filter(|block| block.speaker == "You")
            .count(),
        2
    );
    assert!(model.account.as_ref().unwrap().recovery.is_none());
    finish(&mut model, &mut session);
}

#[test]
fn failed_resubmission_cancel_and_abandon_keep_the_right_text() {
    let (mut model, mut session, _, release) = running();
    enqueue(&mut model, &mut session, "recover me");
    model.editor.insert("previous draft");
    model.editor.left();
    let cursor = model.editor.cursor;
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    account::input(&mut model, Key::Escape, &mut session);
    account::input(&mut model, Key::Enter, &mut session); // Cancellation rejects enqueue.
    assert_eq!(model.editor.text, "recover me");
    assert!(model.account.as_ref().unwrap().recovery.is_some());
    assert!(
        model
            .account
            .as_ref()
            .unwrap()
            .local_notice
            .contains("draft kept")
    );
    release.send(()).unwrap();
    finish(&mut model, &mut session);
    assert_eq!(model.editor.text, "recover me");
    account::input(&mut model, Key::AbandonRecovered, &mut session);
    assert_eq!(model.editor.text, "previous draft");
    assert_eq!(model.editor.cursor, cursor);
    assert!(
        model
            .account
            .as_ref()
            .unwrap()
            .local_notice
            .contains("discarded")
    );
    assert!(session.pending_guidance().snapshot().is_empty());
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    assert_eq!(model.editor.text, "previous draft");
}

#[test]
fn delivery_win_and_empty_retries_do_not_replace_a_draft() {
    let (mut model, mut session, _, release) = running();
    enqueue(&mut model, &mut session, "claimed guidance");
    release.send(()).unwrap();
    loop {
        let event = session::tests::next(&mut session);
        let claimed = matches!(event, Event::Guidance { .. });
        account::event(&mut model, event);
        if claimed {
            break;
        }
    }
    model.editor.insert("kept draft");
    model.editor.left();
    let cursor = model.editor.cursor;
    for _ in 0..2 {
        account::input(&mut model, Key::RetrieveQueued, &mut session);
        assert_eq!(model.editor.text, "kept draft");
        assert_eq!(model.editor.cursor, cursor);
        assert!(model.account.as_ref().unwrap().recovery.is_none());
    }
    assert!(
        model
            .account
            .as_ref()
            .unwrap()
            .local_notice
            .contains("No pending")
    );
    finish(&mut model, &mut session);
}

#[test]
fn slash_prefixed_recovery_submits_as_guidance_not_a_command() {
    let (mut model, mut session, _, release) = running();
    assert!(session.enqueue("/help"));
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    assert!(model.menu.pasted_literal);
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(session.pending_guidance().snapshot(), ["/help"]);
    assert!(model.editor.text.is_empty());
    assert!(
        !model
            .blocks
            .iter()
            .any(|block| block.text.contains("↑↓ choose"))
    );
    release.send(()).unwrap();
    finish(&mut model, &mut session);
    assert!(
        model
            .blocks
            .iter()
            .any(|block| block.speaker == "You" && block.text == "/help")
    );
}

#[test]
fn slash_recovery_is_literal_and_logout_retains_both_drafts() {
    let (mut model, mut session, _, release) = running();
    assert!(session.enqueue("/help"));
    model.editor.insert("prior draft");
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    assert_eq!(model.editor.text, "/help");
    assert!(model.menu.pasted_literal);
    assert!(session.logout());
    model.account.as_mut().unwrap().signing_out();
    release.send(()).unwrap();
    loop {
        let event = session::tests::next(&mut session);
        let done = matches!(event, Event::LoggedOut);
        account::event(&mut model, event);
        if done {
            break;
        }
    }
    assert_eq!(model.editor.text, "/help");
    assert!(model.account.as_ref().unwrap().recovery.is_some());
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.editor.text, "/help");
    assert_eq!(
        model
            .blocks
            .iter()
            .filter(|block| block.speaker == "You")
            .count(),
        1
    );
    account::input(&mut model, Key::AbandonRecovered, &mut session);
    assert_eq!(model.editor.text, "prior draft");
}

#[test]
fn failed_delivery_marks_other_guidance_unsent_and_keeps_recovery() {
    let (mut model, mut session, _, release) = running_with_failure(true);
    enqueue(&mut model, &mut session, "undelivered guidance");
    enqueue(&mut model, &mut session, "recovered guidance");
    model.editor.insert("prior draft");
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    release.send(()).unwrap();
    finish(&mut model, &mut session);
    assert_eq!(model.editor.text, "recovered guidance");
    assert!(model.account.as_ref().unwrap().recovery.is_some());
    assert!(model.blocks.iter().any(|block| block.speaker == "Status"
        && block.text == "Queued message was not sent:\nundelivered guidance"));
    assert!(session.pending_guidance().snapshot().is_empty());
    assert!(
        !view::chrome(&model, 80, 24)
            .iter()
            .any(|row| row.text.contains("queued") || row.text.contains("Alt+↑ edit"))
    );
    account::input(&mut model, Key::AbandonRecovered, &mut session);
    assert_eq!(model.editor.text, "prior draft");
}

#[test]
fn active_menu_keeps_its_navigation_and_hides_recovery_hint() {
    let (mut model, mut session, _, release) = running();
    assert!(session.enqueue("pending message"));
    account::input(&mut model, Key::Text("/".into()), &mut session);
    assert!(model.menu.active(&model.editor.text));
    assert!(
        !view::chrome(&model, 80, 24)
            .iter()
            .any(|row| row.text.contains("Alt+↑ edit"))
    );
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    assert_eq!(session.pending_guidance().snapshot(), ["pending message"]);
    account::input(&mut model, Key::Up, &mut session);
    assert!(model.menu.selected > 0);
    assert_eq!(model.editor.text, "/");
    release.send(()).unwrap();
    finish(&mut model, &mut session);
}

#[test]
fn pending_previews_are_bounded_and_do_not_reflow_transcript_on_resize() {
    let (mut model, mut session, _, release) = running();
    let mut layout = view::Layout::default();
    let initial = layout.frame(&model, 80, 24);
    let transcript = initial
        .iter()
        .take_while(|row| !row.transient)
        .map(|row| row.text.clone())
        .collect::<Vec<_>>();
    for index in 0..8 {
        enqueue(
            &mut model,
            &mut session,
            &format!("guidance {index}\n  detail"),
        );
    }
    for (width, height) in [(25, 9), (40, 12), (80, 24), (25, 9)] {
        let frame = layout.frame(&model, width, height);
        let rows = view::chrome(&model, width, height);
        assert!(rows.len() < height);
        assert_eq!(
            rows.iter().filter(|row| row.text.starts_with('─')).count(),
            2
        );
        assert_eq!(rows.last().unwrap().tone, style::Tone::Muted);
        assert!(rows.iter().all(|row| text::width(&row.text) < width));
        assert_eq!(
            frame
                .iter()
                .take_while(|row| !row.transient)
                .map(|row| row.text.clone())
                .collect::<Vec<_>>(),
            transcript
        );
        assert!(rows.iter().any(|row| row.text.contains("queued")));
        assert!(rows.iter().any(|row| row.text.contains("guidance")));
    }
    let rows = view::chrome(&model, 80, 24);
    assert!(rows.iter().any(|row| row.text.contains("more")));
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    assert_eq!(model.editor.text, "guidance 7\n  detail");
    assert!(
        !view::chrome(&model, 80, 24)
            .iter()
            .any(|row| row.text.contains("Alt+↑ edit"))
    );
    release.send(()).unwrap();
    finish(&mut model, &mut session);
}
