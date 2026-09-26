use super::*;
use crate::{
    providers::openai_account::{
        self as account, FailureCode, FailureEvent, Progress, ProviderFailure, Request, Response,
        client::{self, Attempt, Delivery, RequestStage},
    },
    tls::Budget,
};
use std::{ops::ControlFlow, sync::atomic::AtomicUsize};

struct ReportedFailure {
    calls: Arc<AtomicUsize>,
    failure: ProviderFailure,
    compaction: bool,
}
impl worker::Backend for ReportedFailure {
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
        progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        assert_eq!(
            request.instructions.starts_with("Summarize"),
            self.compaction
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            progress(Progress::Attempt(Attempt {
                connection_attempt: 1,
                delivery: Delivery::Streaming,
                stage: Some(RequestStage::ResponseRead),
                response_status: Some(200),
                stream_events: 3,
                provider_failure: Some(self.failure),
                diagnostic: Some(account::Error::RemoteFailure(self.failure).to_string()),
                ..Default::default()
            }))
            .is_continue()
        );
        Err(client::Error::Response {
            error: account::Error::RemoteFailure(self.failure),
            delivery: Delivery::Streaming,
        })
    }
}

#[test]
fn worker_records_provider_failure_in_generation_and_both_compaction_paths() {
    for mode in ["generation", "manual", "automatic"] {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else {
            continue;
        };
        let mut history =
            persistence::create_in(&store, Model::Luna, Some(&fixture.0), None).unwrap();
        let id = history.record.as_ref().unwrap().id().to_owned();
        if mode == "manual" {
            history.begin("prior completed task".into()).unwrap();
            history.turns[0].end = Some(End::Complete);
            history.checkpoint().unwrap();
        } else if mode == "automatic" {
            history.projection.limit_bytes = 65536;
            for index in 0..10 {
                history
                    .begin(format!("{index}: {}", "x".repeat(7000)))
                    .unwrap();
                history.turns[index].end = Some(End::Complete);
                history.checkpoint().unwrap();
            }
        }
        let failure = ProviderFailure {
            event: if mode == "generation" {
                FailureEvent::ResponseFailed
            } else {
                FailureEvent::Error
            },
            code: if mode == "generation" {
                FailureCode::ServerError
            } else {
                FailureCode::RateLimitExceeded
            },
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let mut run = Session::with_history(
            Model::Luna,
            ReportedFailure {
                calls: Arc::clone(&calls),
                failure,
                compaction: mode != "generation",
            },
            None,
            history,
        )
        .unwrap();
        assert!(matches!(tests::next(&mut run), Event::Restored { .. }));
        assert!(matches!(tests::next(&mut run), Event::Ready));
        if mode == "manual" {
            assert!(run.compact());
        } else {
            assert!(run.submit("new task"));
        }
        loop {
            match tests::next(&mut run) {
                Event::Finished(
                    End::Failed(Failure::Account(client::Error::Response {
                        error: account::Error::RemoteFailure(actual),
                        delivery: Delivery::Streaming,
                    })),
                    _,
                ) => {
                    assert_eq!(actual, failure);
                    break;
                }
                Event::Finished(end, _) => panic!("unexpected outcome: {end:?}"),
                Event::ToolStarted { .. } => panic!("failed response authorized a tool"),
                _ => {}
            }
        }
        drop(run);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let saved = persistence::load(&store, &id, true).unwrap();
        if mode == "generation" {
            assert_eq!(
                saved.history.turns[0].steps[0].attempts[0].provider_failure,
                Some(failure)
            );
            assert!(saved.history.turns[0].steps[0].response.is_none());
        } else {
            assert_eq!(
                saved.history.projection.failed_attempts[0].provider_failure,
                Some(failure)
            );
            assert!(saved.history.projection.failed);
        }
    }
}
