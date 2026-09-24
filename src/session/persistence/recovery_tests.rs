use super::*;

#[test]
fn interrupted_stream_and_prior_receipt_survive_resume_without_replay() {
    use crate::providers::openai_account::client::{Attempt, Delivery, RequestStage};
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history = create(&store, Model::Luna, None).unwrap();
    let id = history.record.as_ref().unwrap().id.clone();
    history.begin("finish the workspace task".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(session::tool_tests::calls_response(vec![
            session::tool_tests::call("read-1", "read_file", r#"{"path":"notes.txt"}"#),
        ])),
        accepted: true,
        results: vec![Receipt {
            call_id: "read-1".into(),
            output: r#"{"text":"prior result"}"#.into(),
            summary: "Read notes.txt".into(),
        }],
        ..Default::default()
    });
    history.turns[0].steps.push(Step {
        text: "unfinished answer".into(),
        attempts: vec![Attempt {
            delivery: Delivery::Streaming,
            stage: Some(RequestStage::ResponseRead),
            operation: Some("TLS record body read".into()),
            category: Some("ConnectionReset".into()),
            os_code: Some(10054),
            accepted_wire_bytes: 91,
            diagnostic: Some("synthetic connection reset / response read".into()),
            retrying: false,
        }],
        ..Default::default()
    });
    history.turns[0].end = Some(End::Failed(Failure::Worker));
    history.turns[0].outcome = "Response interrupted; remote completion uncertain".into();
    history.turns[0].metrics.requests = 1;
    history.turns[0].metrics.connection_attempts = 1;
    history.turns[0].metrics.submissions = 1;
    history.checkpoint().unwrap();
    drop(history);

    let saved = load(&store, &id, true).unwrap();
    let step = &saved.history.turns[0].steps[1];
    assert_eq!(step.text, "unfinished answer");
    assert_eq!(step.attempts[0].delivery, Delivery::Streaming);
    assert_eq!(step.attempts[0].os_code, Some(10054));
    assert!(step.response.is_none());
    assert_eq!(
        (
            saved.history.turns[0].metrics.requests,
            saved.history.turns[0].metrics.connection_attempts,
            saved.history.turns[0].metrics.submissions
        ),
        (1, 1, 1)
    );
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut run =
        Session::with_history(Model::Luna, Backend(observed.clone()), None, saved.history).unwrap();
    assert!(matches!(
        session::tests::next(&mut run),
        Event::Restored { .. }
    ));
    assert!(matches!(session::tests::next(&mut run), Event::Ready));
    assert!(observed.lock().unwrap().is_empty());
    assert!(run.submit("continue from the recorded state"));
    loop {
        match session::tests::next(&mut run) {
            Event::Finished(End::Complete, metrics) => {
                assert_eq!(metrics.requests, 1);
                break;
            }
            Event::Text(_) => {}
            _ => panic!("historical tool was replayed"),
        }
    }
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("prior result"));
    assert!(requests[0].contains("unfinished answer"));
    assert!(requests[0].contains("streaming_unvalidated"));
    assert!(requests[0].contains("continue from the recorded state"));
}
