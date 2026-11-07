//! An outcome on its way to the model: a requested stop, saving, a failed display and compaction.

use super::*;

#[test]
fn a_stop_the_user_asked_for_reaches_the_next_request_without_starting_one() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("start", "sleep 20")])),
        (200, completion("Started.", vec![])),
        (200, completion("Understood.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.run_turn("Run it", &mut |_| Ok(())).unwrap();
    let observer = agent.processes();
    let id = observer.processes().unwrap()[0].id.clone();
    observer.request_stop(&id).unwrap();
    wait_for(|| !observer.completions_since(0).1.is_empty());
    assert!(!agent.can_continue_processes());
    agent.run_turn("What happened?", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(deliveries(&requests[2].body), 1);
    assert!(crate::test_support::latest_state(&requests[2].body).contains(": stopped after "));
}

#[test]
fn saving_keeps_an_outcome_no_request_has_carried_and_delivery_adds_none() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![start("start", &gate("go"))])),
        (200, completion("Started.", vec![])),
        (200, completion("Seen.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.enable_sessions(home.path()).unwrap();
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    release(&directory, "go");
    wait_for(|| agent.tools.process_completion_waiting());
    agent.save_session().unwrap();
    let records = |agent: &Agent| {
        agent
            .sessions()
            .unwrap()
            .snapshot()
            .records()
            .iter()
            .filter(|record| matches!(record, crate::sessions::Record::ProcessFinished { .. }))
            .count()
    };
    assert_eq!(records(&agent), 1);
    // Saved is not delivered: the model still hears about it, and nothing doubles.
    assert!(agent.can_continue_processes());
    agent.run_turn("And now?", &mut |_| Ok(())).unwrap();
    assert_eq!(records(&agent), 1);
    let requests = fixture.finish();
    assert_eq!(deliveries(&requests[2].body), 1);
}

#[test]
fn a_failed_display_keeps_the_outcome_owed_and_saves_every_completion() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![start("one", &gate("go")), start("two", &gate("go"))],
            ),
        ),
        (200, completion("Started.", vec![])),
        (200, completion("Lost with the display.", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    release(&directory, "go");
    wait_for(|| agent.processes().completions_since(0).1.len() == 2);
    let result = agent.continue_processes(&mut |event| match event {
        Event::ProcessFinished { .. } => Err("The display closed".into()),
        _ => Ok(()),
    });
    assert!(result.is_err());
    assert_eq!(saved_finishes(&agent), 2);
    assert!(agent.tools.process_completion_waiting());
    fixture.finish();
}

#[test]
fn a_summary_of_portions_keeps_an_outcome_only_state_carried() {
    let directory = Directory::new();
    let rejected = Value::object([(
        "error",
        Value::object([
            ("code", Value::string("context_length_exceeded")),
            ("message", Value::string("Maximum context length exceeded")),
        ]),
    )]);
    // A rejection compacts by portions, however many this prompt takes.
    let mut responses = vec![
        (200, completion("", vec![start("start", &gate("go"))])),
        (200, completion("Started.", vec![])),
        (200, completion("It passed.", vec![])),
        (400, rejected),
    ];
    responses.extend((0..8).map(|_| (200, completion("Kept.", vec![]))));
    let fixture = HttpFixture::new(responses);
    let mut agent = agent(&directory, &fixture.endpoint);
    agent.set_auto_compact(90);
    agent.run_turn("Build it", &mut |_| Ok(())).unwrap();
    release(&directory, "go");
    wait_for(|| agent.can_continue_processes());
    agent.continue_processes(&mut |_| Ok(())).unwrap();
    let delivered = agent.context.states.last().unwrap().before;
    // Larger than the recent window, so the boundary falls after the state.
    let fixed = agent.current_system_message().encode().len()
        + crate::tools::encoded_size()
        + agent.request_environment().len();
    let prompt = "next objective ".repeat(fixed * 4 / 15);
    agent.run_turn(&prompt, &mut |_| Ok(())).unwrap();
    assert!(agent.context.from > delivered);
    let requests = fixture.finish_received();
    assert!(
        requests
            .iter()
            .filter(|request| request.body.get("tools").is_none())
            .any(|request| request.body.encode().contains(SECTION))
    );
}
