use super::*;

#[test]
fn documented_sol_efforts_are_selectable_and_sent_to_responses() {
    let (server, endpoint) = serve(
        200,
        r#"{"object":"list","data":[{"id":"gpt-6.1-sol"},{"id":"unknown-model"}]}"#,
    );
    let mut api = Api::fixture_api_key("fixture-api-key", &endpoint);
    let models = Service::catalog(&api).unwrap();
    let sol = models
        .iter()
        .find(|model| model.id == "gpt-6.1-sol")
        .unwrap();
    assert_eq!(
        sol.efforts,
        [
            Effort::Default,
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::Xhigh,
            Effort::Max,
        ]
    );
    assert_eq!(models[1].efforts, [Effort::Default]);
    let sol_backend = OpenAi::with_api(api.clone(), "gpt-6.1-sol".into()).unwrap();
    assert_eq!(sol_backend.maintenance_effort(Effort::High), Effort::High);
    assert_eq!(
        sol_backend.maintenance_effort(Effort::Ultra),
        Effort::Default
    );
    let unknown_backend = OpenAi::with_api(api.clone(), "unknown-model".into()).unwrap();
    assert_eq!(
        unknown_backend.maintenance_effort(Effort::High),
        Effort::Default
    );
    server.finish().unwrap();

    let response = "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Done\"}]}]}}\n\n";
    let (server, endpoint) = serve(200, response);
    api.base_url = endpoint;
    let backend = OpenAi::with_api(api, "gpt-6.1-sol".into()).unwrap();
    let completion = backend
        .sample_with_effort(Effort::High, &[], false, None, None, &mut |_| Ok(()))
        .unwrap();
    assert_eq!(completion.text, "Done");
    let (_, body) = server.finish().unwrap();
    let request = crate::json::parse(&body).unwrap();
    assert_eq!(
        request
            .get("reasoning")
            .and_then(|reasoning| reasoning.get("effort"))
            .and_then(Value::as_str),
        Some("high")
    );
}

#[test]
fn api_key_route_lists_public_models_without_inventing_capabilities() {
    let (server, endpoint) = serve(
        200,
        r#"{"object":"list","data":[{"id":"model-a","object":"model","owned_by":"openai"},{"id":"model-a"},{"id":"model-b"},{"id":"bad model"}]}"#,
    );
    let api = Api::fixture_api_key("fixture-api-key", &endpoint);
    assert_eq!(Service::id(&api), "openai-api");
    let models = Service::catalog(&api).unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["model-a", "model-b"]
    );
    assert!(
        models
            .iter()
            .all(|model| model.efforts == [Effort::Default])
    );
    let backend = OpenAi::with_api(api, "model-a".into()).unwrap();
    assert_eq!(Backend::id(&backend), "openai-api");
    assert!(Backend::known_limits(&backend).is_none());
    assert_eq!(
        Backend::maintenance_effort(&backend, Effort::High),
        Effort::Default
    );
    let (headers, body) = server.finish().unwrap();
    assert!(headers[0].starts_with("GET /models "));
    assert!(
        headers
            .iter()
            .any(|line| line == "Authorization: Bearer fixture-api-key")
    );
    assert!(!headers.iter().any(|line| line.starts_with("session-id:")));
    assert!(body.is_empty());
}

#[test]
fn api_key_route_streams_tools_and_replays_the_roundtrip_without_subscription_headers() {
    let first = concat!(
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"namespace\":\"jecode\",\"call_id\":\"call_1\",\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\\\"README.md\\\"}\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\n",
    );
    let (server, endpoint) = serve(200, first);
    let api = Api::fixture_api_key("fixture-api-key", &endpoint);
    let mut backend = OpenAi::with_api(api, "model-a".into()).unwrap();
    Backend::set_effort(&mut backend, Effort::High);
    Backend::set_conversation(&mut backend, Some("session-1".into()));
    let user = Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Read README")),
    ]);
    let completion = Backend::sample(
        &backend,
        std::slice::from_ref(&user),
        true,
        Some(1024),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(completion.calls.len(), 1);
    assert_eq!(completion.calls[0].name, "read");
    let (headers, body) = server.finish().unwrap();
    assert!(
        headers
            .iter()
            .any(|line| line == "Authorization: Bearer fixture-api-key")
    );
    assert!(!headers.iter().any(|line| line.starts_with("session-id:")));
    let request = crate::json::parse(&body).unwrap();
    assert_eq!(request.get("store"), Some(&Value::Bool(false)));
    assert_eq!(request.get("stream"), Some(&Value::Bool(true)));
    assert_eq!(request.get("max_output_tokens"), Some(&Value::number(1024)));
    assert_eq!(
        request
            .get("reasoning")
            .and_then(|reasoning| reasoning.get("effort"))
            .and_then(Value::as_str),
        Some("high")
    );
    assert_eq!(
        request.get("prompt_cache_key").and_then(Value::as_str),
        Some("session-1")
    );
    assert_eq!(
        request.get("tools").and_then(Value::as_array).unwrap()[0]
            .get("type")
            .and_then(Value::as_str),
        Some("namespace")
    );
    let account = Backend::account(&backend).unwrap();
    assert!(account.starts_with("key-sha256:"));
    assert!(!account.contains("fixture-api-key"));
    assert_eq!(
        completion
            .message
            .get("jecode_origin")
            .unwrap()
            .get("backend")
            .and_then(Value::as_str),
        Some("openai-api")
    );
    assert_eq!(
        completion
            .message
            .get("jecode_origin")
            .unwrap()
            .get("account")
            .and_then(Value::as_str),
        Some(account.as_str())
    );

    let second = "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Done\"}]}]}}\n\n";
    let (server, endpoint) = serve(200, second);
    backend.api.base_url = endpoint;
    let tool = Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string("call_1")),
        ("content", Value::string("README contents")),
    ]);
    let done = Backend::sample(
        &backend,
        &[user, completion.message, tool],
        true,
        None,
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(done.text, "Done");
    let (_, body) = server.finish().unwrap();
    let request = crate::json::parse(&body).unwrap();
    let input = request.get("input").and_then(Value::as_array).unwrap();
    assert!(
        input
            .iter()
            .any(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
    );
    assert!(
        input
            .iter()
            .any(
                |item| item.get("type").and_then(Value::as_str) == Some("function_call_output")
                    && item.get("output").and_then(Value::as_str) == Some("README contents")
            )
    );
}

#[test]
fn api_key_route_redacts_provider_error_and_scopes_distinct_keys() {
    assert!(Api::with_api_key("bad key".into()).is_err());
    let (server, endpoint) = serve(
        401,
        r#"{"error":{"code":"invalid_api_key","message":"fixture-api-key rejected"}}"#,
    );
    let api = Api::fixture_api_key("fixture-api-key", &endpoint);
    let other = Api::with_api_key("other-fixture-key".into()).unwrap();
    assert_ne!(api.account(), other.account());
    let error = Service::check_credential(&api).unwrap_err();
    assert!(error.contains("HTTP 401"));
    assert!(!error.contains("fixture-api-key"));
    assert!(error.contains("[redacted]"));
    server.finish().unwrap();
}

#[test]
fn replacing_an_api_key_keeps_the_transport_and_uses_the_new_identity() {
    let (server, endpoint) = serve(200, r#"{"data":[{"id":"fixture-model"}]}"#);
    let api = Api::fixture_api_key("fixture-original-key", &endpoint);
    let updated = Service::with_credential(&api, "fixture-updated-key".into()).unwrap();
    assert_eq!(updated.id(), "openai-api");
    assert_ne!(updated.account(), Service::account(&api));
    assert_eq!(updated.catalog().unwrap()[0].id, "fixture-model");
    let (headers, _) = server.finish().unwrap();
    assert!(
        headers
            .iter()
            .any(|line| line == "Authorization: Bearer fixture-updated-key")
    );
    assert!(
        !headers
            .iter()
            .any(|line| line.contains("fixture-original-key"))
    );
}
