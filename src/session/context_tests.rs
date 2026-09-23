use super::*;
use crate::providers::openai_account::{Progress, Response, client};
use crate::session::{End, Session, history::Step, tests};
use crate::tls::NetworkError;
use std::sync::{Arc, Mutex};

struct Summarizer {
    requests: Arc<Mutex<Vec<String>>>,
    fail: bool,
    catalog: Option<crate::providers::openai_account::catalog::Catalog>,
}
impl Backend for Summarizer {
    fn login(
        &mut self,
        _: &Budget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn catalog(
        &mut self,
        _: &Budget<'_>,
    ) -> Result<Option<crate::providers::openai_account::catalog::Catalog>, client::Error> {
        Ok(self.catalog.clone())
    }
    fn generate(
        &mut self,
        request: &Request,
        _: &Budget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.requests
            .lock()
            .unwrap()
            .push(request.encode(MAX_CONTEXT)?);
        if request.instructions.starts_with("Summarize") {
            assert!(request.tools.is_empty());
            if self.fail {
                return Err(NetworkError::io(
                    crate::tls::IoOperation::ReadRecordHeader,
                    &std::io::Error::from(std::io::ErrorKind::ConnectionReset),
                )
                .into());
            }
            Ok(tests::response(
                "Retain the user's original goal and completed change in settings.rs.",
                Status::Completed,
            ))
        } else {
            Ok(tests::response("Continued", Status::Completed))
        }
    }
}
fn history() -> History {
    let mut history = History::default();
    history.projection.limit_bytes = 65536;
    for n in 0..4 {
        history.begin(format!("task-{n}")).unwrap();
        let answer = "a".repeat(28_000);
        let turn = history.turns.last_mut().unwrap();
        turn.steps.push(Step {
            text: answer.clone(),
            response: Some(tests::response(&answer, Status::Completed)),
            accepted: true,
            ..Default::default()
        });
        turn.end = Some(End::Complete);
        turn.outcome = "Complete".into();
    }
    history
}
fn finish(session: &mut Session) -> End {
    loop {
        match tests::next(session) {
            Event::Finished(end, _) => return end,
            Event::ContextReport(_) | Event::Text(_) | Event::RequestStarted => {}
            _ => panic!("unexpected compaction event"),
        }
    }
}
#[test]
fn compaction_projects_a_summary_and_keeps_recent_turns_without_tools() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let selected = Model::new("account-model", Some("low")).unwrap();
    let mut session = Session::with_history(
        selected,
        Summarizer {
            requests: observed.clone(),
            fail: false,
            catalog: None,
        },
        None,
        history(),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert_eq!(finish(&mut session), End::Complete);
    assert!(session.submit("continue"));
    assert_eq!(finish(&mut session), End::Complete);
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        let value = crate::json::parse(request, Default::default()).unwrap();
        assert_eq!(
            value.get("model").and_then(crate::json::Value::text),
            Some("account-model")
        );
        assert_eq!(
            value
                .get("reasoning")
                .and_then(|v| v.get("effort"))
                .and_then(crate::json::Value::text),
            Some("low")
        );
    }
    assert!(requests[0].contains("task-0") && requests[0].contains("task-1"));
    assert!(!requests[0].contains("task-2"));
    assert!(!requests[1].contains("task-0"));
    assert!(
        requests[1].contains("settings.rs")
            && requests[1].contains("task-2")
            && requests[1].contains("task-3")
    );
}
#[test]
fn automatic_compaction_and_followup_share_the_selected_pair() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let selected = Model::new("account-model", Some("high")).unwrap();
    let mut session = Session::with_history(
        selected,
        Summarizer {
            requests: observed.clone(),
            fail: false,
            catalog: None,
        },
        None,
        history(),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.submit("continue"));
    assert_eq!(finish(&mut session), End::Complete);
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        let value = crate::json::parse(request, Default::default()).unwrap();
        assert_eq!(
            value.get("model").and_then(crate::json::Value::text),
            Some("account-model")
        );
        assert_eq!(
            value
                .get("reasoning")
                .and_then(|v| v.get("effort"))
                .and_then(crate::json::Value::text),
            Some("high")
        );
    }
}
#[test]
fn failed_compaction_is_not_automatically_retried_and_original_projection_remains() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        Model::Luna,
        Summarizer {
            requests: observed.clone(),
            fail: true,
            catalog: None,
        },
        None,
        history(),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert!(matches!(
        finish(&mut session),
        End::Failed(Failure::Account(_))
    ));
    assert!(session.submit("new task"));
    assert_eq!(finish(&mut session), End::Failed(Failure::HistoryLimit));
    assert_eq!(observed.lock().unwrap().len(), 1);
    assert!(session.inspect_context());
    let Event::ContextReport(report) = tests::next(&mut session) else {
        panic!("missing context report")
    };
    assert!(report.contains("5 canonical turns / 0 summarized"));
    assert!(report.contains("unknown input tokens"));
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn fresh_unsupported_pair_rejects_manual_compaction_without_a_checkpoint() {
    let catalog = crate::providers::openai_account::catalog::Catalog::parse(br#"{"models":[{"slug":"replacement","visibility":"list","supported_reasoning_levels":[{"effort":"high"}]}]}"#).unwrap();
    for selected in [Model::Luna, Model::new("replacement", Some("low")).unwrap()] {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else { return };
        let mut saved = crate::session::persistence::create(&store, selected, None).unwrap();
        let mut prepared = history();
        saved.turns = std::mem::take(&mut prepared.turns);
        saved.projection = prepared.projection;
        saved.checkpoint().unwrap();
        let id = saved.record.as_ref().unwrap().id().to_owned();
        let sessions = store.directory("sessions").unwrap();
        let filename = format!("{id}.json");
        let before = sessions.read(&filename, 2 * 1024 * 1024).unwrap().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut session = Session::with_history(
            selected,
            Summarizer {
                requests: requests.clone(),
                fail: false,
                catalog: Some(catalog.clone()),
            },
            None,
            saved,
        )
        .unwrap();
        assert!(matches!(tests::next(&mut session), Event::Restored { .. }));
        assert!(matches!(tests::next(&mut session), Event::CatalogLoaded(_)));
        assert!(matches!(tests::next(&mut session), Event::Ready));
        assert!(session.selection_unavailable());
        assert!(!session.submit("keep this draft"));
        assert!(!session.compact());
        assert!(session.ready());
        assert!(session.poll().is_none());
        assert!(requests.lock().unwrap().is_empty());
        assert_eq!(
            sessions.read(&filename, 2 * 1024 * 1024).unwrap().unwrap(),
            before
        );
        assert!(session.inspect_context());
        let Event::ContextReport(report) = tests::next(&mut session) else {
            panic!("missing context report")
        };
        assert!(
            report.contains("4 canonical turns / 0 summarized"),
            "{report}"
        );

        let replacement = Model::new("replacement", Some("high")).unwrap();
        assert!(session.set_model(replacement));
        assert!(
            matches!(tests::next(&mut session), Event::ModelChanged(model) if model == replacement)
        );
        assert!(session.compact());
        assert_eq!(finish(&mut session), End::Complete);
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn unknown_effort_capability_does_not_block_manual_compaction() {
    let catalog = crate::providers::openai_account::catalog::Catalog::parse(
        br#"{"models":[{"slug":"known-model","visibility":"list"}]}"#,
    )
    .unwrap();
    let selected = Model::new("known-model", Some("high")).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        selected,
        Summarizer {
            requests: requests.clone(),
            fail: false,
            catalog: Some(catalog),
        },
        None,
        history(),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::CatalogLoaded(_)));
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(!session.selection_unavailable());
    assert!(session.compact());
    assert_eq!(finish(&mut session), End::Complete);
    assert_eq!(requests.lock().unwrap().len(), 1);
}
