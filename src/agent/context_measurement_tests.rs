use super::*;
use crate::{
    backend::Usage,
    test_support::{Directory, HttpFixture, completion},
};

fn measured_reply(input: usize, output: usize) -> Value {
    let mut reply = completion("Finished.", vec![]);
    if let Value::Object(fields) = &mut reply {
        fields.insert(
            "usage".into(),
            Value::object([
                ("prompt_tokens", Value::number(input)),
                ("completion_tokens", Value::number(output)),
            ]),
        );
    }
    reply
}

#[test]
fn arrivals_during_inference_are_not_absorbed_into_the_measured_prefix() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, measured_reply(1000, 2000))]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_limits(500_000, None);
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    let messages = Arc::clone(&agent.messages);
    let receipt = Value::object([
        ("role", Value::string("user")),
        (
            "content",
            Value::string("Collaborator returned while inference was in flight.".repeat(100)),
        ),
    ]);
    let mut appended = false;
    agent
        .run_turn("Inspect.", &mut |event| {
            if matches!(event, Event::Waiting { .. }) && !appended {
                messages.lock().unwrap().push(receipt.clone());
                appended = true;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(agent.context.measured_end, 2);
    assert_eq!(agent.context.output_message, Some(3));
    let source = agent.messages.lock().unwrap();
    assert_eq!(
        agent
            .context
            .estimate_growth(&source, 1000, agent.client.inputs()),
        3000 + agent
            .context
            .estimate_bytes(crate::context::portable(&receipt).encode().len())
    );
    drop(source);
    let requests = fixture.finish();
    assert!(!requests[0].body.encode().contains("Collaborator returned"));
}

#[test]
fn dynamic_instructions_change_the_estimate_without_discarding_usage() {
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent
        .permissions
        .set_level(crate::permissions::Level::Workspace)
        .unwrap();
    agent.prepare_turn("Inspect.").unwrap();
    let request = agent.transport_request();
    agent.context.record_request(
        Some(&Usage {
            input_tokens: Some(1000),
            ..Usage::default()
        }),
        request.measurement,
    );
    let before = agent.locked_estimate();
    agent
        .permissions
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    let after = agent.locked_estimate();
    assert!(after < before);
    assert_eq!(agent.context.input_tokens, Some(1000));
    assert_eq!(agent.context.measured_end, 2);
    let request = agent.transport_request();
    assert_eq!(request.estimated_tokens, after);
    assert_eq!(agent.context_status().tokens, None);
}

#[test]
fn generated_output_does_not_define_the_compaction_threshold() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, measured_reply(100_000, 75_000))]);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.fixture_limits(500_000, None);
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent.set_auto_compact(75);
    let mut generated = 0;
    agent
        .run_turn("Inspect.", &mut |event| {
            match event {
                Event::Generated { tokens } => generated += tokens,
                Event::Maintenance { .. } | Event::ContextCompacted { .. } => {
                    panic!("Output is below the input-context budget")
                }
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(generated, 75_000);
    assert_eq!(crate::context::threshold(500_000, 75), 375_000);
    assert_eq!(agent.context_status().tokens, Some(175_000));
    assert!(agent.locked_estimate() < 375_000);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
#[ignore = "uses ChatGPT Terra at max effort for two bounded synthetic read turns"]
fn live_max_effort_context_meter_follows_provider_usage() {
    use crate::backend::{
        Backend, Service,
        openai::{Api, OpenAi},
    };
    let store = crate::config::Store::discover().unwrap();
    let session = crate::auth::openai::Store::new(store.path().parent().unwrap().to_path_buf())
        .active()
        .unwrap()
        .expect("No active ChatGPT registration");
    let api = Api::new(session).unwrap();
    api.catalog().unwrap();
    let mut backend = OpenAi::with_api(api, "gpt-6.1-terra".into()).unwrap();
    backend.set_effort(crate::effort::Effort::Max);
    let directory = Directory::new();
    let source: String = (0..150)
        .map(|i| format!("row {i:03}: {}\n", "synthetic evidence ".repeat(20)))
        .collect();
    for path in ["first.txt", "second.txt"] {
        std::fs::write(directory.path().join(path), &source).unwrap();
    }
    let mut agent = Agent::with_backend(Box::new(backend), Tools::new(directory.path()).unwrap());
    agent.set_auto_compact(75);
    let mut total_requests = 0;
    for path in ["first.txt", "second.txt"] {
        let mut last = None;
        let mut requests = Vec::new();
        let prompt = format!(
            "Read {path} once using read with limit 200. Then reply with exactly JECODE_CONTEXT_OK and nothing else. Do not call other tools."
        );
        agent
            .run_turn(prompt, &mut |event| {
                match event {
                    Event::Context(status) => last = Some(status),
                    Event::Waiting { .. } => {
                        if total_requests >= 4 {
                            return Err("Synthetic probe request limit reached".into());
                        }
                        total_requests += 1;
                        requests.push(last.unwrap().tokens);
                    }
                    Event::Recovering { .. } | Event::RequestDiscarded => {
                        return Err("Unexpected live probe recovery".into());
                    }
                    Event::Maintenance { .. } | Event::ContextCompacted { .. } => {
                        return Err("Unexpected live probe compaction".into());
                    }
                    _ => {}
                }
                Ok(())
            })
            .unwrap();
        eprintln!(
            "{path}: provider counts before requests {requests:?}; final meter {:?}; provider input {:?}, output {:?}; capacity {:?}",
            agent.context_status().tokens,
            agent.context.input_tokens,
            agent.context.output_tokens,
            agent.context_status().capacity
        );
        assert_eq!(requests.len(), 2);
        assert!(agent.context.input_tokens.is_some());
        assert_eq!(
            agent.context_status().tokens,
            Some(agent.context.input_tokens.unwrap() + agent.context.output_tokens.unwrap())
        );
    }
}

#[test]
fn changing_endpoints_forgets_the_previous_rejection_ceiling() {
    let directory = Directory::new();
    let mut client = OpenRouter::fixture("http://127.0.0.1:1".into());
    client.fixture_limits(500_000, None);
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent.context.ceiling = Some(100_000);
    assert_eq!(
        agent.context.capacity(agent.client.known_limits()),
        Some(100_000)
    );
    assert_eq!(agent.context_status().capacity, Some(500_000));
    let next = if agent.routing() == crate::backend::Routing::Auto {
        crate::backend::Routing::Default
    } else {
        crate::backend::Routing::Auto
    };
    agent.set_routing(next);
    assert_eq!(agent.context_status().capacity, Some(500_000));
}

#[test]
fn known_native_output_survives_projection_resets_but_stays_model_scoped() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, measured_reply(1000, 75_000))]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.run_turn("Inspect.", &mut |_| Ok(())).unwrap();
    agent.context.clear_measurement();
    assert!(agent.locked_estimate() >= 75_000);
    assert_eq!(agent.context_status().tokens, Some(76_000));
    agent.set_model("fixture/other".into()).unwrap();
    assert!(agent.locked_estimate() < 75_000);
    assert_eq!(agent.context_status().tokens, None);
    agent.set_model("fixture/model".into()).unwrap();
    assert!(agent.locked_estimate() >= 75_000);
    assert_eq!(agent.context_status().tokens, None);
    assert_eq!(fixture.finish().len(), 1);
}
