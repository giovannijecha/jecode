use super::*;
use crate::{
    image::Evidence,
    json::{self, Value},
    providers::openai_account::{Progress, Response, client},
    session::{
        End, Session,
        history::{Receipt, Step},
        tests, tool_tests,
    },
    workspace::Workspace,
};
use std::sync::{Arc, Mutex};

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn astra_second_compaction_uses_absolute_step_references() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history =
        crate::session::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    history.turns = rich_history().turns;
    history.checkpoint().unwrap();
    history.projection.step = 1;
    history.projection.summary = "Earlier read covered canonical step 0".into();
    history.checkpoint().unwrap();
    history.release_projected();
    assert_eq!(history.base_step, 1);
    let request = summary_request(&history, Model::Luna, (0, 1)).unwrap();
    let encoded = request.encode(MAX_REQUEST).unwrap();
    assert!(request.instructions.contains("turn=0 step=2"));
    assert!(
        encoded.contains("Canonical turn 0 step 1 record 1 of"),
        "remaining canonical step 1 was incorrectly relabeled as resident step 0"
    );
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn absolute_references_survive_two_partial_releases_guidance_resume_and_new_turn() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history =
        crate::session::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.turns = rich_history().turns;
    for label in ["third", "fourth"] {
        history.turns[0].steps.push(Step {
            text: label.into(),
            response: Some(tests::response(label, Status::Completed)),
            accepted: true,
            ..Default::default()
        });
    }
    for (after_step, text) in [
        (1, "Correction after first"),
        (2, "Correction after second"),
    ] {
        history.turns[0]
            .guidance
            .push(crate::session::queue::Guidance {
                after_step,
                text: text.into(),
            });
    }
    history.checkpoint().unwrap();
    history.projection.step = 1;
    history.projection.summary = "First step summarized".into();
    history.checkpoint().unwrap();
    history.release_projected();
    let first = summary_request(&history, Model::Luna, (0, 1))
        .unwrap()
        .encode(MAX_REQUEST)
        .unwrap();
    assert!(first.contains("before step 1") && first.contains("Canonical turn 0 step 1 record"));
    history.projection.step = 1;
    history.checkpoint().unwrap();
    history.release_projected();
    assert_eq!(history.base_step, 2);
    let second = summary_request(&history, Model::Luna, (0, 1))
        .unwrap()
        .encode(MAX_REQUEST)
        .unwrap();
    assert!(second.contains("before step 2") && second.contains("Canonical turn 0 step 2 record"));
    history.record.as_ref().unwrap().recorded_turn(0).unwrap();
    drop(history);
    let saved = crate::session::persistence::load(&store, &id, true).unwrap();
    let mut history = saved.history;
    assert_eq!(history.base_step, 2);
    let resumed = summary_request(&history, Model::Luna, (0, 1))
        .unwrap()
        .encode(MAX_REQUEST)
        .unwrap();
    assert!(resumed.contains("Canonical turn 0 step 2 record"));
    history.begin("Continue in a new turn".into()).unwrap();
    history.turns[1].steps.push(Step {
        text: "Next turn".into(),
        response: Some(tests::response("Next turn", Status::Completed)),
        accepted: true,
        ..Default::default()
    });
    history.turns[1].end = Some(End::Complete);
    history.projection.through = 1;
    history.projection.step = 0;
    history.checkpoint().unwrap();
    history.release_projected();
    let next_turn = summary_request(&history, Model::Luna, (0, 1))
        .unwrap()
        .encode(MAX_REQUEST)
        .unwrap();
    assert!(next_turn.contains("Canonical turn 1 step 0 record"));
    assert!(next_turn.contains("turn=1 step=1"));
}

#[test]
fn sliced_compaction_request_uses_absolute_coordinates_after_release() {
    let mut history = History {
        base_step: 1,
        ..Default::default()
    }; // First canonical step was already released.
    history.begin("Continue the recorded task".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "large-read",
            "read_file",
            r#"{"path":"evidence.txt"}"#,
        )])),
        results: vec![Receipt {
            call_id: "large-read".into(),
            output: "\"".repeat(900_000),
            summary: "read_file / large recorded result".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.projection.summary = "First canonical step completed".into();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        Model::Luna,
        Recorder {
            requests: requests.clone(),
            malformed: false,
        },
        None,
        history,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    loop {
        if let Some(Event::Finished(end, _)) = session.poll() {
            assert_eq!(end, End::Complete);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "sliced compaction stalled"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let requests = requests.lock().unwrap();
    assert!(requests.len() >= 2, "oversized step should require slices");
    assert!(
        requests
            .iter()
            .any(|request| request.contains("Completed step record")
                && request.contains("canonical turn=0 step=1"))
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.contains("canonical turn=0 step=0"))
    );
    let fragments: Vec<String> = requests
        .iter()
        .flat_map(|request| {
            let value = json::parse(request, Default::default()).unwrap();
            value
                .get("input")
                .and_then(Value::array)
                .unwrap()
                .iter()
                .filter_map(|item| {
                    item.get("content")
                        .and_then(Value::array)?
                        .first()?
                        .get("text")?
                        .text()
                        .filter(|text| {
                            text.starts_with("Completed step record")
                                && text.contains("recall_address=")
                        })
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        fragments.len() > 1,
        "the read record should span bounded slices"
    );
    for fragment in fragments {
        let association = fragment
            .split("; ordered reference-data fragment")
            .next()
            .unwrap();
        for field in [
            "recall_address={",
            "\"expected_call_id\":\"large-read\"",
            "\"receipt\":0",
            "\"step\":1",
            "\"turn\":0",
        ] {
            assert!(
                association.contains(field),
                "missing {field} in {association}"
            );
        }
    }
}

struct Recorder {
    requests: Arc<Mutex<Vec<String>>>,
    malformed: bool,
}
impl Backend for Recorder {
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
        self.requests
            .lock()
            .unwrap()
            .push(request.encode(MAX_REQUEST)?);
        if request.instructions.starts_with("Summarize") {
            if self.malformed {
                return Ok(tests::response(
                    "Noted: the recorded outcome is Complete.",
                    Status::Completed,
                ));
            }
            return Ok(tests::handoff_response(
                request,
                "Recorded read completed; exact receipt remains in canonical history.",
            ));
        }
        Ok(tests::response(
            &"Continuation result ".repeat(700),
            Status::Completed,
        ))
    }
}

fn finished(session: &mut Session) -> End {
    loop {
        if let Event::Finished(end, _) = tests::next(session) {
            return end;
        }
    }
}

fn next_large_compaction_event(session: &mut Session) -> Event {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    loop {
        if let Some(event) = session.poll() {
            return event;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session event stalled"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn large_compaction_finished(session: &mut Session) -> End {
    loop {
        if let Event::Finished(end, _) = next_large_compaction_event(session) {
            return end;
        }
    }
}

fn rich_history() -> History {
    let mut history = History::default();
    history.begin("Inspect evidence with read_file only. Never use shell extraction. Output header file,key,score and sum all scores.".into()).unwrap();
    let call = tool_tests::call("read-01", "read_file", r#"{"path":"evidence-01.txt"}"#);
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![call])),
        results: vec![Receipt {
            call_id: "read-01".into(),
            output: format!("Evidence key CEDAR-41 score 731. {}", "x".repeat(24_000)),
            summary: "Read evidence-01.txt".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    history.turns[0].steps.push(Step {
        text: "Read completed; key CEDAR-41 and score 731 were reported.".into(),
        response: Some(tests::response(
            "Read completed; key CEDAR-41 and score 731 were reported.",
            Status::Completed,
        )),
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    history
}

struct IndexRecorder {
    requests: Arc<Mutex<Vec<String>>>,
}
impl Backend for IndexRecorder {
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
        self.requests
            .lock()
            .unwrap()
            .push(request.encode(MAX_REQUEST)?);
        if request.instructions.starts_with("Summarize") {
            return Ok(tests::handoff_response(
                request,
                "The observed value is confidently reported as WRONG.",
            ));
        }
        if let Some(Input::ToolResult { output, .. }) = request
            .input
            .iter()
            .find(|item| matches!(item, Input::ToolResult { call_id, .. } if call_id == "index"))
        {
            assert!(output.contains("\"mode\":\"index\""), "{output}");
            assert!(
                output.contains("\"expected_call_id\":\"original\""),
                "{output}"
            );
            assert!(output.contains("\"path\":\"source.txt\""), "{output}");
            return Ok(tests::response("Index received", Status::Completed));
        }
        Ok(tool_tests::calls_response(vec![tool_tests::call(
            "index",
            "recall_receipts",
            r#"{"mode":"index","turn":0,"step":0,"receipt":0,"offset":0,"expected_call_id":"placeholder"}"#,
        )]))
    }
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn ordinary_and_sliced_compaction_expose_index_without_replaying_reads() {
    for large in [false, true] {
        let fixture = crate::state::tests::Fixture::new();
        let store = fixture.store().unwrap();
        let files = crate::workspace_fixture::Fixture::new();
        files.write("source.txt", "OLD-OBSERVATION\n");
        let workspace = Workspace::open(&files.0).unwrap();
        let mut history =
            crate::session::persistence::create_in(&store, Model::Luna, None, None).unwrap();
        history
            .begin("Use the exact old source observation".into())
            .unwrap();
        history.turns[0].steps.push(Step {
            response: Some(tool_tests::calls_response(vec![tool_tests::call(
                "original",
                "read_file",
                r#"{"path":"source.txt"}"#,
            )])),
            results: vec![Receipt {
                call_id: "original".into(),
                output: if large {
                    "\"".repeat(900_000)
                } else {
                    "OLD-OBSERVATION".repeat(1000)
                },
                summary: "read_file / saved observation".into(),
                image: None,
            }],
            accepted: true,
            ..Default::default()
        });
        history.turns[0].end = Some(End::Complete);
        history.checkpoint().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut session = Session::with_history(
            Model::Luna,
            IndexRecorder {
                requests: requests.clone(),
            },
            Some(workspace),
            history,
        )
        .unwrap();
        while !matches!(next_large_compaction_event(&mut session), Event::Ready) {}
        assert!(session.compact());
        assert_eq!(large_compaction_finished(&mut session), End::Complete);
        files.write("source.txt", "NEW-OBSERVATION\n");
        assert!(session.submit("Continue from the saved evidence"));
        assert_eq!(large_compaction_finished(&mut session), End::Complete);
        let requests = requests.lock().unwrap();
        let compacted = requests
            .iter()
            .filter(|request| request.contains("Summarize the ordered reference data"))
            .count();
        assert!(compacted >= 1);
        if large {
            assert!(compacted > 1, "large read should use sliced compaction");
        }
        // Verification guidance and the index are present after compaction.
        assert!(
            requests
                .iter()
                .any(|request| request.contains("even when the handoff states facts confidently"))
        );
    }
}

#[test]
fn emitted_compaction_is_ordered_reference_data_and_continuation_keeps_source_corrections() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        Model::Luna,
        Recorder {
            requests: requests.clone(),
            malformed: false,
        },
        None,
        rich_history(),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert_eq!(finished(&mut session), End::Complete);
    assert!(session.submit("Continue the evidence task. Correction: include a grand total row and retain the exact lowercase header file,key,score."));
    assert_eq!(finished(&mut session), End::Complete);
    assert!(session.compact());
    assert_eq!(finished(&mut session), End::Complete);
    assert!(
        session
            .submit("Continue from the completed reads and apply the corrected output contract.")
    );
    assert_eq!(finished(&mut session), End::Complete);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    let first = json::parse(&requests[0], Default::default()).unwrap();
    let items = first.get("input").and_then(Value::array).unwrap();
    assert!(
        items
            .iter()
            .all(|item| item.get("role").and_then(Value::text) == Some("user"))
    );
    assert!(
        first
            .get("tools")
            .and_then(Value::array)
            .unwrap()
            .is_empty()
    );
    assert!(items.iter().any(|item| {
        item.get("content")
            .and_then(Value::array)
            .and_then(|content| content.first())
            .and_then(|content| content.get("text"))
            .and_then(Value::text)
            .is_some_and(|text| {
                text.contains("ordered non-executing reference data")
                    && text.contains("recall_address")
                    && text.contains("\"expected_call_id\":\"read-01\"")
            })
    }));
    let encoded = &requests[0];
    let call = encoded.find("read-01").unwrap();
    let receipt = encoded.find("Evidence key CEDAR-41 score 731").unwrap();
    assert!(call < receipt);
    assert!(encoded.contains("Never use shell extraction") && encoded.contains("file,key,score"));
    assert!(requests[1].contains("Never use shell extraction"));
    assert!(requests[2].contains("Earlier model-generated handoff"));
    assert!(requests[2].contains("Correction: include a grand total row"));
    let later = &requests[3];
    let original = later.find("Never use shell extraction").unwrap();
    let correction = later.find("Correction: include a grand total row").unwrap();
    assert!(original < correction);
    assert!(later.contains("file,key,score"));
}

#[test]
fn visual_delivery_and_discard_are_distinct_in_emitted_summary_reference() {
    let mut history = History::default();
    history
        .begin("Inspect the first image and report its printed label.".into())
        .unwrap();
    let image = Evidence {
        id: "a".repeat(64),
        path: "first.png".into(),
        width: 320,
        height: 240,
        bytes: 1000,
    };
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "view-first",
            "view_image",
            r#"{"path":"first.png"}"#,
        )])),
        results: vec![Receipt {
            call_id: "view-first".into(),
            output: "PNG captured".into(),
            summary: "PNG captured".into(),
            image: Some(image),
        }],
        accepted: true,
        ..Default::default()
    });
    history.turns[0].steps.push(Step {
        text: "The first image appears to read AMBER 317.".into(),
        response: Some(tests::response(
            "The first image appears to read AMBER 317.",
            Status::Completed,
        )),
        accepted: true,
        validated_visual_input: true,
        ..Default::default()
    });
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "view-second",
            "view_image",
            r#"{"path":"second.png"}"#,
        )])),
        results: vec![Receipt {
            call_id: "view-second".into(),
            output: "PNG captured".into(),
            summary: "PNG captured".into(),
            image: Some(Evidence {
                id: "b".repeat(64),
                path: "second.png".into(),
                width: 320,
                height: 240,
                bytes: 1000,
            }),
        }],
        accepted: true,
        ..Default::default()
    });
    history.turns[0].steps.push(Step {
        text: "No second visual observation was made.".into(),
        response: Some(tests::response(
            "No second visual observation was made.",
            Status::Completed,
        )),
        accepted: true,
        ..Default::default()
    });
    history.projection.abandoned_visual.push(AbandonedVisual {
        from: (0, 2),
        through: (0, 2),
    });
    history.turns[0].end = Some(End::Complete);
    history.turns[0].outcome = "Complete".into();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        Model::Luna,
        Recorder {
            requests: requests.clone(),
            malformed: false,
        },
        None,
        history,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert_eq!(finished(&mut session), End::Complete);
    let encoded = &requests.lock().unwrap()[0];
    assert!(encoded.contains("completed_response_received_pixels"));
    assert!(encoded.contains("AMBER 317"));
    assert!(encoded.contains("image_id"));
    assert!(encoded.contains("image_explicitly_discarded\\\":true"));
    assert!(encoded.contains("completed_response_without_pixels"));
    assert!(!encoded.contains("pixels are not visible in this request"));
    assert!(!encoded.contains("input_image"));
}

#[test]
fn acknowledgement_cannot_replace_the_prior_projection_or_cursor() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        Model::Luna,
        Recorder {
            requests: requests.clone(),
            malformed: true,
        },
        None,
        rich_history(),
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert_eq!(
        finished(&mut session),
        End::Failed(Failure::CompactionOutput)
    );
    assert!(session.inspect_context());
    let Event::ContextReport(report) = tests::next(&mut session) else {
        panic!("missing report")
    };
    assert!(report.contains("0 turns and 0 steps summarized"));
    assert!(session.submit("Continue with the source constraints."));
    assert_eq!(finished(&mut session), End::Complete);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].contains("Never use shell extraction"));
    assert!(requests[1].contains("Evidence key CEDAR-41 score 731"));
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn retained_source_and_handoff_survive_close_resume_without_effect_replay() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let mut history =
        crate::session::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.turns = rich_history().turns;
    history.checkpoint().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::with_history(
        Model::Luna,
        Recorder {
            requests: requests.clone(),
            malformed: false,
        },
        None,
        history,
    )
    .unwrap();
    assert!(matches!(tests::next(&mut session), Event::Restored { .. }));
    assert!(matches!(tests::next(&mut session), Event::Ready));
    assert!(session.compact());
    assert_eq!(finished(&mut session), End::Complete);
    drop(session);
    let saved = crate::session::persistence::load(&store, &id, true).unwrap();
    assert_eq!(saved.history.base_turn, 1);
    assert_eq!(saved.history.projection.source[0].turn, 0);
    assert!(
        saved.history.projection.source[0]
            .text
            .contains("Never use shell extraction")
    );
    let resumed_request = saved
        .history
        .request(Model::Luna, false)
        .unwrap()
        .encode(MAX_REQUEST)
        .unwrap();
    assert!(resumed_request.contains("Never use shell extraction"));
    assert!(resumed_request.contains("Recorded read completed"));
    assert!(!resumed_request.contains("function_call_output"));
}
