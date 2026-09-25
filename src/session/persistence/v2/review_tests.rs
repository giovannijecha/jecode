use super::*;
use crate::session::{
    End,
    history::{Receipt, Step},
};

#[test]
fn astra_live_compaction_persists_the_current_turn_completion() {
    use crate::{
        providers::openai_account::{Progress, Request, Response, Status, client},
        session::{self, Event, Session},
        tls::Budget,
    };
    use std::ops::ControlFlow;
    struct Answers;
    impl session::worker::Backend for Answers {
        fn login(
            &mut self,
            _: &Budget<'_>,
            _: &mut dyn FnMut(&str) -> ControlFlow<()>,
        ) -> Result<(), client::Error> {
            Ok(())
        }
        fn generate(
            &mut self,
            request: &Request,
            _: &Budget<'_>,
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            let answer = if request.instructions.starts_with("Summarize") {
                "Prior task completed.".to_owned()
            } else {
                "DONE ".to_owned() + &"y".repeat(100_000)
            };
            Ok(session::tests::response(&answer, Status::Completed))
        }
    }
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.projection.limit_bytes = 65536;
    history.begin("older task".into()).unwrap();
    let text = "x".repeat(100_000);
    history.turns[0].steps.push(Step {
        text: text.clone(),
        response: Some(session::tests::response(&text, Status::Completed)),
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    let mut run = Session::with_history(Model::Luna, Answers, None, history).unwrap();
    assert!(matches!(
        session::tests::next(&mut run),
        Event::Restored { .. }
    ));
    assert!(matches!(session::tests::next(&mut run), Event::Ready));
    let mut completed_metrics = Vec::new();
    for prompt in ["new task 1", "new task 2", "new task 3"] {
        assert!(run.submit(prompt));
        loop {
            if let Event::Finished(end, metrics) = session::tests::next(&mut run) {
                assert_eq!(end, End::Complete);
                completed_metrics.push(metrics);
                break;
            }
        }
    }
    drop(run);
    let saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(saved.history.base_turn, 3);
    let canonical = page(saved.history.record.as_ref().unwrap(), 0, 4).unwrap();
    for (index, prompt) in ["new task 1", "new task 2", "new task 3"]
        .into_iter()
        .enumerate()
    {
        let turn = &canonical[index + 1];
        assert_eq!(turn.prompt, prompt);
        assert_eq!(turn.end, Some(End::Complete));
        assert_eq!(turn.outcome, "Complete");
        assert_eq!(turn.metrics.requests, completed_metrics[index].requests);
        assert!(turn.metrics.requests >= 2);
    }
}

#[test]
fn astra_resume_interrupted_then_new_turn_can_resume_again() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("interrupted task".into()).unwrap();
    history.checkpoint().unwrap();
    drop(history);
    let mut saved = super::super::load(&store, &id, true).unwrap();
    assert!(saved.history.turns[0].end.is_some());
    saved.history.begin("continue".into()).unwrap();
    saved.history.checkpoint().unwrap();
    saved.history.turns[1].end = Some(End::Complete);
    saved.history.turns[1].outcome = "Complete".into();
    saved.history.checkpoint().unwrap();
    drop(saved);
    let again = super::super::load(&store, &id, true);
    assert!(again.is_ok(), "second resume failed: {:?}", again.err());
}

#[test]
fn controller_resume_commits_interruption_and_never_replays_old_receipts() {
    use crate::{
        providers::openai_account::{Progress, Request, Response, Status, client},
        session::{self, Event, Session},
        tls::Budget,
    };
    use std::{ops::ControlFlow, sync::Arc};
    struct Answer(Arc<std::sync::atomic::AtomicUsize>);
    impl session::worker::Backend for Answer {
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
            _: &Budget<'_>,
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(session::tests::response(
                "continued safely",
                Status::Completed,
            ))
        }
    }
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history
        .begin("interrupted with a recorded receipt".into())
        .unwrap();
    let call = session::tool_tests::call("old-read", "read_file", r#"{"path":"old.txt"}"#);
    history.turns[0].steps.push(Step {
        response: Some(session::tool_tests::calls_response(vec![call])),
        accepted: true,
        results: vec![Receipt {
            call_id: "old-read".into(),
            output: "EXACT-OLD-RECEIPT".into(),
            summary: "Read old file".into(),
            image: None,
        }],
        ..Default::default()
    });
    history.checkpoint().unwrap();
    drop(history);
    let saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(
        saved.history.turns[0].end,
        Some(End::Failed(session::Failure::Worker))
    );
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut run =
        Session::with_history(Model::Luna, Answer(Arc::clone(&count)), None, saved.history)
            .unwrap();
    match session::tests::next(&mut run) {
        Event::Restored { items, turns, .. } => {
            assert_eq!(turns, 1);
            assert!(items.iter().any(|item| item.text == "Read old file"));
        }
        _ => panic!("missing restored event"),
    }
    assert!(matches!(session::tests::next(&mut run), Event::Ready));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert!(run.submit("continue with a new task"));
    let finished = loop {
        match session::tests::next(&mut run) {
            Event::Finished(end, metrics) => break (end, metrics),
            Event::EditPlanned { .. } | Event::ToolStarted { .. } => {
                panic!("historical tool was replayed")
            }
            _ => {}
        }
    };
    assert_eq!(finished.0, End::Complete);
    assert_eq!(finished.1.requests, 1);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    drop(run);
    let again = super::super::load(&store, &id, true).unwrap();
    assert_eq!(again.turns, 2);
    let canonical = page(again.history.record.as_ref().unwrap(), 0, 2).unwrap();
    assert_eq!(
        canonical[0].end,
        Some(End::Failed(session::Failure::Worker))
    );
    assert_eq!(canonical[0].steps[0].results[0].output, "EXACT-OLD-RECEIPT");
    assert_eq!(canonical[1].prompt, "continue with a new task");
    assert_eq!(canonical[1].end, Some(End::Complete));
    assert_eq!(canonical[1].metrics.requests, finished.1.requests);
}

#[test]
fn controller_outcomes_and_metrics_stay_with_their_v2_turns() {
    use crate::{
        providers::openai_account::{Progress, Request, Response, Status, client},
        session::{self, Event, Failure, Session},
        tls::{Budget, NetworkError},
    };
    use std::{ops::ControlFlow, sync::mpsc, time::Duration};
    struct Outcomes {
        calls: usize,
        waiting: mpsc::Sender<()>,
    }
    impl session::worker::Backend for Outcomes {
        fn login(
            &mut self,
            _: &Budget<'_>,
            _: &mut dyn FnMut(&str) -> ControlFlow<()>,
        ) -> Result<(), client::Error> {
            Ok(())
        }
        fn generate(
            &mut self,
            request: &Request,
            budget: &Budget<'_>,
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            if request.instructions.starts_with("Summarize") {
                return Ok(session::tests::response(
                    "Earlier completed and interrupted work was summarized.",
                    Status::Completed,
                ));
            }
            self.calls += 1;
            match self.calls {
                1 | 3 => Ok(session::tests::response(
                    &format!("completed {}", "x".repeat(100_000)),
                    Status::Completed,
                )),
                2 => {
                    self.waiting.send(()).unwrap();
                    while !budget.cancelled.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(NetworkError::Cancelled.into())
                }
                4 => Ok(session::tool_tests::calls_response(vec![
                    session::tool_tests::call(
                        "unexpected",
                        "read_file",
                        r#"{"path":"unknown.txt"}"#,
                    ),
                ])),
                _ => panic!("unexpected model generation"),
            }
        }
    }
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.projection.limit_bytes = 65536;
    history
        .begin("earlier large completed task".into())
        .unwrap();
    let text = "o".repeat(100_000);
    history.turns[0].steps.push(Step {
        text: text.clone(),
        response: Some(session::tests::response(&text, Status::Completed)),
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history.checkpoint().unwrap();
    let (waiting, entered) = mpsc::channel();
    let mut run =
        Session::with_history(Model::Luna, Outcomes { calls: 0, waiting }, None, history).unwrap();
    assert!(matches!(
        session::tests::next(&mut run),
        Event::Restored { .. }
    ));
    assert!(matches!(session::tests::next(&mut run), Event::Ready));
    let mut finished = Vec::new();
    for (index, prompt) in ["complete one", "cancel", "complete two", "fail"]
        .into_iter()
        .enumerate()
    {
        assert!(run.submit(prompt));
        if index == 1 {
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            run.cancel();
        }
        loop {
            if let Event::Finished(end, metrics) = session::tests::next(&mut run) {
                finished.push((end, metrics));
                break;
            }
        }
    }
    assert_eq!(finished[0].0, End::Complete);
    assert_eq!(finished[1].0, End::Failed(Failure::Cancelled));
    assert_eq!(finished[2].0, End::Complete);
    assert_eq!(finished[3].0, End::Failed(Failure::UnexpectedTools));
    drop(run);
    let saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(saved.history.base_turn, 4);
    let canonical = page(saved.history.record.as_ref().unwrap(), 0, 5).unwrap();
    for (index, turn) in canonical.iter().skip(1).enumerate() {
        // The existing decoder represents a saved failed end generically;
        // the exact outcome text and metrics remain canonical per turn.
        let stored_end = match finished[index].0 {
            End::Failed(_) => End::Failed(Failure::Worker),
            end => end,
        };
        assert_eq!(turn.end, Some(stored_end));
        assert_eq!(turn.metrics.requests, finished[index].1.requests);
        assert_eq!(turn.metrics.tool_calls, finished[index].1.tool_calls);
        assert_eq!(turn.metrics.elapsed_ms, finished[index].1.elapsed_ms);
        assert_eq!(turn.metrics.first_text_ms, finished[index].1.first_text_ms);
        assert!(turn.metrics.requests >= 1);
    }
    assert_eq!(canonical[1].outcome, "Complete");
    assert_eq!(canonical[2].outcome, Failure::Cancelled.to_string());
    assert_eq!(canonical[3].outcome, "Complete");
    assert_eq!(canonical[4].outcome, Failure::UnexpectedTools.to_string());
}

#[test]
fn astra_compaction_checkpoint_with_guidance_resumes_before_next_write() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("task with guidance".into()).unwrap();
    history.turns[0]
        .guidance
        .push(crate::session::queue::Guidance {
            after_step: 0,
            text: "use the selected approach".into(),
        });
    history.turns[0]
        .guidance
        .push(crate::session::queue::Guidance {
            after_step: 1,
            text: "keep this boundary guidance".into(),
        });
    history.turns[0].steps.push(Step {
        text: "completed first step".into(),
        response: Some(crate::session::tests::response(
            "completed first step",
            crate::providers::openai_account::Status::Completed,
        )),
        accepted: true,
        ..Default::default()
    });
    history.checkpoint().unwrap();
    history.projection.step = 1;
    history.projection.summary = "First step and guidance summarized".into();
    history.checkpoint().unwrap();
    let info = head::read(&store.directory("sessions-v2").unwrap(), &id).unwrap();
    assert_eq!((info.step, info.guidance_base), (1, 1));
    history.release_projected();
    drop(history);
    let again = super::super::load(&store, &id, true).unwrap();
    assert_eq!(again.history.base_step, 1);
    assert_eq!(again.history.base_guidance, 1);
    assert_eq!(again.history.turns[0].guidance.len(), 1);
    assert_eq!(
        again.history.turns[0].guidance[0].text,
        "keep this boundary guidance"
    );
    assert_eq!(again.history.turns[0].guidance[0].after_step, 0);
    let full = page(again.history.record.as_ref().unwrap(), 0, 1).unwrap();
    assert_eq!(full[0].guidance.len(), 2);
    assert_eq!(full[0].guidance[0].text, "use the selected approach");
    assert_eq!(full[0].guidance[1].text, "keep this boundary guidance");
}

#[test]
fn controller_compaction_commits_boundary_guidance_before_close() {
    use crate::{
        providers::openai_account::{Progress, Request, Response, Status, client},
        session::{self, Event, Session},
        tls::Budget,
    };
    use std::ops::ControlFlow;
    struct Summary;
    impl session::worker::Backend for Summary {
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
            _: &Budget<'_>,
            _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
        ) -> Result<Response, client::Error> {
            Ok(session::tests::response(
                "First completed step and its guidance were summarized.",
                Status::Completed,
            ))
        }
    }
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("guided work".into()).unwrap();
    history.turns[0]
        .guidance
        .push(crate::session::queue::Guidance {
            after_step: 0,
            text: "before the step".into(),
        });
    history.turns[0].steps.push(Step {
        text: "first completed result".into(),
        response: Some(session::tests::response(
            "first completed result",
            Status::Completed,
        )),
        accepted: true,
        ..Default::default()
    });
    history.turns[0]
        .guidance
        .push(crate::session::queue::Guidance {
            after_step: 1,
            text: "at the next step".into(),
        });
    history.checkpoint().unwrap();
    let mut run = Session::with_history(Model::Luna, Summary, None, history).unwrap();
    assert!(matches!(
        session::tests::next(&mut run),
        Event::Restored { .. }
    ));
    assert!(matches!(session::tests::next(&mut run), Event::Ready));
    assert!(run.compact());
    loop {
        if let Event::Finished(end, metrics) = session::tests::next(&mut run) {
            assert_eq!(end, End::Complete);
            assert_eq!(metrics.requests, 1);
            break;
        }
    }
    drop(run);
    let saved = super::super::load(&store, &id, true).unwrap();
    assert_eq!(saved.history.base_step, 1);
    assert_eq!(saved.history.base_guidance, 1);
    assert_eq!(saved.history.turns[0].guidance.len(), 1);
    assert_eq!(saved.history.turns[0].guidance[0].text, "at the next step");
    let full = page(saved.history.record.as_ref().unwrap(), 0, 1).unwrap();
    assert_eq!(full[0].guidance.len(), 2);
    assert_eq!(full[0].guidance[0].text, "before the step");
    assert_eq!(full[0].guidance[1].text, "at the next step");
}
