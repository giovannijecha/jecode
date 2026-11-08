use super::*;

#[test]
fn openai_api_key_usage_has_a_documented_window_after_catalog_discovery() {
    let directory = Directory::new();
    let catalog = Value::object([(
        "data",
        Value::Array(vec![Value::object([("id", Value::string("gpt-6.1-sol"))])]),
    )]);
    let answer = Value::object([
        ("type", Value::string("response.completed")),
        (
            "response",
            Value::object([
                ("status", Value::string("completed")),
                (
                    "output",
                    Value::Array(vec![Value::object([
                        ("type", Value::string("message")),
                        ("role", Value::string("assistant")),
                        (
                            "content",
                            Value::Array(vec![Value::object([
                                ("type", Value::string("output_text")),
                                ("text", Value::string("Done.")),
                            ])]),
                        ),
                    ])]),
                ),
                (
                    "usage",
                    Value::object([
                        ("input_tokens", Value::number(275_000)),
                        ("output_tokens", Value::number(11_600)),
                        ("total_tokens", Value::number(286_600)),
                    ]),
                ),
            ]),
        ),
    ]);
    let fixture = HttpFixture::streaming(vec![
        crate::test_support::Response::Json(200, catalog),
        crate::test_support::Response::Stream(vec![(
            std::time::Duration::ZERO,
            format!("data: {}\n\n", answer.encode()),
        )]),
    ]);
    let api = crate::backend::openai::Api::fixture_api_key(
        "isolated-fixture-api-key",
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    let client = crate::backend::openai::OpenAi::with_api(api, "gpt-6.1-sol".into()).unwrap();
    let mut agent = Agent::new(client, Tools::new(directory.path()).unwrap());
    let mut observed = Vec::new();
    agent
        .run_turn("Inspect.", &mut |event| {
            if let Event::Context(status) = event {
                observed.push(status);
            }
            Ok(())
        })
        .unwrap();
    let status = agent.context_status();
    assert_eq!(status.tokens, Some(286_600));
    assert_eq!(status.capacity, Some(1_050_000));
    assert_eq!(status.percent(), Some(27));
    assert!(observed.iter().any(|status| status.tokens == Some(286_600)));
    let requests = fixture.finish();
    assert!(requests[0].headers[0].starts_with("GET /models "));
    assert!(requests[1].headers[0].starts_with("POST /responses "));
}
