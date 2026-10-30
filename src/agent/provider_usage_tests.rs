use super::*;
use crate::{
    backend::openai::{Api, OpenAi},
    test_support::{Directory, HttpFixture, Response, completion, tool_call},
};
use std::time::Duration;

fn openai_client(fixture: &HttpFixture) -> OpenAi {
    let api = Api::fixture(
        crate::auth::openai::Session::fixture("isolated-access-token"),
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    OpenAi::with_api(api, "fixture-model".into()).unwrap()
}

fn metered_reply() -> Value {
    let mut response = completion("Done", vec![]);
    if let Value::Object(fields) = &mut response {
        fields.insert(
            "usage".into(),
            Value::object([
                ("prompt_tokens", Value::number(1000)),
                ("completion_tokens", Value::number(75_000)),
            ]),
        );
    }
    response
}

#[test]
fn replacing_an_openrouter_key_clears_only_the_previous_credentials_usage() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, metered_reply()), (200, metered_reply())]);
    let client = OpenRouter::fixture(fixture.endpoint.clone());
    let api = client.api();
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    agent.run_turn("Inspect.", &mut |_| Ok(())).unwrap();
    let identity = agent.identity();
    assert_eq!(agent.context_status().tokens, Some(76_000));
    agent
        .replace_backend(Box::new(
            OpenRouter::with_api(
                api.clone().with_key("isolated-fixture-key".into()).unwrap(),
                "fixture/model".into(),
            )
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(agent.identity(), identity);
    assert_eq!(agent.context_status().tokens, Some(76_000));
    let next_api = api.with_key("different-isolated-key".into()).unwrap();
    agent
        .replace_backend(Box::new(
            OpenRouter::with_api(next_api, "fixture/model".into()).unwrap(),
        ))
        .unwrap();
    assert_ne!(agent.identity(), identity);
    assert_eq!(agent.context_status().tokens, None);
    assert_eq!(agent.context.input_tokens, None);
    assert_eq!(agent.context.calibration, None);
    assert!(agent.locked_estimate() < 75_000);
    let saved = agent.archive().document().encode();
    assert!(!saved.contains("isolated-fixture-key"));
    assert!(!saved.contains("different-isolated-key"));
    agent.run_turn("Continue.", &mut |_| Ok(())).unwrap();
    assert_eq!(agent.context_status().tokens, Some(76_000));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].body.encode().contains("fixture-reasoning"));
}

#[test]
fn resumed_openrouter_usage_requires_the_same_credential() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(200, metered_reply())]);
    let client = OpenRouter::fixture(fixture.endpoint.clone());
    let api = client.api();
    let mut first = Agent::new(client, Tools::new(directory.path()).unwrap());
    first.enable_sessions(home.path()).unwrap();
    first.run_turn("Inspect.", &mut |_| Ok(())).unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);
    for same_key in [true, false] {
        let api = if same_key {
            api.clone()
        } else {
            api.clone()
                .with_key("different-isolated-key".into())
                .unwrap()
        };
        let client = OpenRouter::with_api(api, "fixture/model".into()).unwrap();
        let mut resumed = Agent::new(client, Tools::new(directory.path()).unwrap());
        resumed.enable_sessions(home.path()).unwrap();
        resumed.resume(&id).unwrap();
        assert_eq!(resumed.context_status().tokens, same_key.then_some(76_000));
        assert_eq!(resumed.locked_estimate() >= 75_000, same_key);
    }
    assert_eq!(fixture.finish().len(), 1);
}

fn sse(kind: &str, response: Value) -> String {
    format!(
        "data: {}\n\n",
        Value::object([("type", Value::string(kind)), ("response", response),]).encode()
    )
}

fn finished(usage: Value) -> String {
    sse(
        "response.completed",
        Value::object([
            ("status", Value::string("completed")),
            ("usage", usage),
            (
                "output",
                Value::Array(vec![Value::object([
                    ("type", Value::string("message")),
                    ("role", Value::string("assistant")),
                    (
                        "content",
                        Value::Array(vec![Value::object([
                            ("type", Value::string("output_text")),
                            ("text", Value::string("Done")),
                        ])]),
                    ),
                ])]),
            ),
        ]),
    )
}

#[test]
fn provider_meter_changes_during_streaming_only_when_usage_arrives() {
    let directory = Directory::new();
    let begin = sse(
        "response.in_progress",
        Value::object([(
            "usage",
            Value::object([
                ("input_tokens", Value::number(500)),
                ("output_tokens", Value::number(0)),
            ]),
        )]),
    );
    let partial = sse(
        "response.in_progress",
        Value::object([(
            "usage",
            Value::object([("output_tokens", Value::number(20))]),
        )]),
    );
    let end = finished(Value::object([
        ("input_tokens", Value::number(500)),
        ("output_tokens", Value::number(20)),
        ("total_tokens", Value::number(530)),
    ]));
    let fixture = HttpFixture::streaming(vec![Response::Stream(vec![
        (Duration::ZERO, begin),
        (Duration::ZERO, "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"Done\"}\n\n".into()),
        (Duration::ZERO, partial), (Duration::ZERO, end),
    ])]);
    let mut agent = Agent::new(
        openai_client(&fixture),
        Tools::new(directory.path()).unwrap(),
    );
    let mut counts = Vec::new();
    let mut text_seen = false;
    let mut count_at_text = None;
    agent
        .run_turn("Inspect.", &mut |event| {
            match event {
                Event::Context(status) if counts.last() != Some(&status.tokens) => {
                    counts.push(status.tokens);
                }
                Event::Streaming { .. } => {
                    text_seen = true;
                    count_at_text = counts.last().copied().flatten();
                }
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert!(text_seen);
    assert_eq!(count_at_text, Some(500));
    assert_eq!(counts, [None, Some(500), Some(520), Some(530)]);
    assert_eq!(agent.context_status().tokens, Some(530));
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn a_new_response_never_borrows_input_tokens_from_the_previous_response() {
    let directory = Directory::new();
    let fixture = HttpFixture::streaming(vec![
        Response::Stream(vec![(
            Duration::ZERO,
            finished(Value::object([
                ("input_tokens", Value::number(100)),
                ("output_tokens", Value::number(20)),
            ])),
        )]),
        Response::Stream(vec![(
            Duration::ZERO,
            finished(Value::object([("output_tokens", Value::number(150))])),
        )]),
    ]);
    let mut agent = Agent::new(
        openai_client(&fixture),
        Tools::new(directory.path()).unwrap(),
    );
    agent.run_turn("First.", &mut |_| Ok(())).unwrap();
    assert_eq!(agent.context_status().tokens, Some(120));
    let mut counts = Vec::new();
    agent
        .run_turn("Second.", &mut |event| {
            if let Event::Context(status) = event {
                counts.push(status.tokens);
            }
            Ok(())
        })
        .unwrap();
    assert!(counts.iter().all(|tokens| *tokens == Some(120)));
    assert_eq!(agent.context_status().tokens, Some(120));
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn tool_results_do_not_change_the_last_provider_count_when_next_usage_is_missing() {
    let directory = Directory::new();
    std::fs::write(
        directory.path().join("source"),
        "large result ".repeat(3000),
    )
    .unwrap();
    let mut first = completion(
        "",
        vec![tool_call(
            "read-1",
            "read",
            Value::object([
                ("path", Value::string("source")),
                ("limit", Value::number(2000)),
            ]),
        )],
    );
    if let Value::Object(fields) = &mut first {
        fields.insert(
            "usage".into(),
            Value::object([
                ("prompt_tokens", Value::number(1000)),
                ("completion_tokens", Value::number(50)),
            ]),
        );
    }
    let fixture = HttpFixture::new(vec![(200, first), (200, completion("Done", vec![]))]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let mut after_tool = false;
    let mut counts = Vec::new();
    agent
        .run_turn("Read source", &mut |event| {
            match event {
                Event::ToolFinished { .. } => after_tool = true,
                Event::Context(status) if after_tool => counts.push(status.tokens),
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert!(!counts.is_empty());
    assert!(counts.iter().all(|tokens| *tokens == Some(1050)));
    assert_eq!(agent.context_status().tokens, Some(1050));
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn nonstreaming_usage_uses_the_same_contract_for_any_backend() {
    use super::backend_tests::fixture::{Account, Reply};
    for backend in ["alpha", "beta"] {
        let directory = Directory::new();
        let account = Account::new(backend, vec![Reply::text("Done")]);
        let mut agent = Agent::new(account.client(), Tools::new(directory.path()).unwrap());
        assert_eq!(agent.context_status().tokens, None);
        agent.run_turn("Inspect.", &mut |_| Ok(())).unwrap();
        assert_eq!(agent.context_status().tokens, Some(77));
        agent.context.clear_measurement();
        assert_eq!(agent.context_status().tokens, Some(77));
    }
}

#[test]
fn resume_restores_the_last_reported_count_without_predicting_the_next_request() {
    let directory = Directory::new();
    let home = Directory::new();
    let mut response = completion("Done", vec![]);
    if let Value::Object(fields) = &mut response {
        fields.insert(
            "usage".into(),
            Value::object([
                ("prompt_tokens", Value::number(7000)),
                ("completion_tokens", Value::number(300)),
            ]),
        );
    }
    let fixture = HttpFixture::new(vec![(200, response)]);
    let mut first = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    first.enable_sessions(home.path()).unwrap();
    first.run_turn("Inspect.", &mut |_| Ok(())).unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);
    let mut resumed = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.context_status().tokens, Some(7300));
    resumed
        .prepare_turn("A newly typed prompt that has never been sent.")
        .unwrap();
    assert_eq!(resumed.context_status().tokens, Some(7300));
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn an_incomplete_response_keeps_reported_usage_when_its_input_returns_to_the_composer() {
    let directory = Directory::new();
    let incomplete = sse(
        "response.incomplete",
        Value::object([
            ("status", Value::string("incomplete")),
            (
                "incomplete_details",
                Value::object([("reason", Value::string("max_output_tokens"))]),
            ),
            (
                "usage",
                Value::object([
                    ("input_tokens", Value::number(500)),
                    ("output_tokens", Value::number(40)),
                ]),
            ),
        ]),
    );
    let fixture =
        HttpFixture::streaming(vec![Response::Stream(vec![(Duration::ZERO, incomplete)])]);
    let mut agent = Agent::new(
        openai_client(&fixture),
        Tools::new(directory.path()).unwrap(),
    );
    let input = agent.input_checkpoint();
    assert!(agent.run_turn("Inspect.", &mut |_| Ok(())).is_err());
    assert_eq!(agent.context_status().tokens, Some(540));
    assert!(agent.return_input(input));
    assert_eq!(agent.context_status().tokens, Some(540));
    assert_eq!(fixture.finish().len(), 1);
}
