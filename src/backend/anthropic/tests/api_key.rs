use super::*;

fn api_key_client(fixture: &HttpFixture, key: &str) -> Api {
    Api::fixture_api_key(key, fixture.endpoint.trim_end_matches("/chat/completions"))
}

#[test]
fn api_key_catalog_and_tool_stream_use_standard_messages_route() {
    let key = "fixture-anthropic-api-key";
    let catalog = crate::json::parse(
        r#"{"data":[{"id":"claude-opus-5-5","display_name":"Claude Opus 5.5","max_input_tokens":1000000,"max_tokens":128000}],"has_more":false}"#,
    )
    .unwrap();
    let reply = stream(
        vec![Value::object([
            ("type", Value::string("tool_use")),
            ("id", Value::string("toolu_api")),
            ("name", Value::string("read")),
            (
                "input",
                Value::object([("path", Value::string("README.md"))]),
            ),
        ])],
        vec![],
        "tool_use",
    );
    let fixture = HttpFixture::streaming(vec![Response::Json(200, catalog), reply]);
    let api = api_key_client(&fixture, key);
    assert_eq!(Service::id(&api), "anthropic-api");
    assert_eq!(
        Service::account(&api).unwrap(),
        crate::auth::key_scope(key, &Cancellation::default()).unwrap()
    );
    assert_eq!(api.catalog().unwrap()[0].id, "claude-opus-5-5");
    let backend = api.create("claude-opus-5-5".into()).unwrap();
    assert_eq!(backend.id(), "anthropic-api");
    assert_eq!(backend.account(), Service::account(&api));
    let directory = crate::test_support::Directory::new();
    let pool = Pool::new(directory.path().join("attachments"));
    let image = pool
        .import_bytes("fixture.png", &crate::attachments::tests::png(4, 3))
        .unwrap();
    let user = crate::attachments::Prompt::new(
        format!("Read README.md and inspect {}", crate::attachments::MARKER),
        vec![image],
    )
    .message();
    let completion = backend
        .sample(&[user], true, Some(100), Some(&pool), &mut |_| Ok(()))
        .unwrap();
    assert_eq!(completion.calls[0].id, "toolu_api");
    let subscription_origin = Origin::with_account("anthropic", backend.model(), backend.account());
    assert!(
        conversation::project(&completion.message, &subscription_origin)
            .get("anthropic_blocks")
            .is_none()
    );
    let requests = fixture.finish();
    assert!(requests[0].headers[0].starts_with("GET /v1/models?limit=100 "));
    assert!(requests[1].headers[0].starts_with("POST /v1/messages HTTP/"));
    for request in &requests {
        assert!(
            request
                .headers
                .iter()
                .any(|line| line.eq_ignore_ascii_case(&format!("authorization: Bearer {key}")))
        );
        assert!(
            request
                .headers
                .iter()
                .any(|line| line.eq_ignore_ascii_case("anthropic-version: 2023-06-01"))
        );
        assert!(!request.headers.iter().any(|line| {
            line.to_ascii_lowercase().starts_with("anthropic-beta:")
                || line.to_ascii_lowercase().starts_with("x-app:")
        }));
        assert!(
            !request
                .headers
                .iter()
                .any(|line| line.to_ascii_lowercase().contains("claude-cli"))
        );
    }
    assert!(
        requests[1]
            .body
            .get("tools")
            .and_then(Value::as_array)
            .is_some()
    );
    let messages = requests[1]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    let content = messages[0]
        .get("content")
        .and_then(Value::as_array)
        .unwrap();
    assert!(
        content
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("image"))
    );
    assert!(!requests[1].body.encode().contains("You are Claude Code"));
    assert!(requests[1].body.get("system").is_none());
}

#[test]
fn api_key_errors_redact_the_key_and_do_not_switch_to_subscription() {
    let key = "fixture-secret-api-key";
    assert!(Api::with_api_key("bad key".into()).is_err());
    let fixture = HttpFixture::new(vec![(401, crate::json::parse(&format!(
        r#"{{"type":"error","error":{{"type":"authentication_error","message":"invalid {key}"}}}}"#
    )).unwrap())]);
    let api = api_key_client(&fixture, key);
    let error = Service::check_credential(&api).unwrap_err();
    assert!(!error.contains(key));
    assert!(error.contains("[redacted]"));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].headers[0].starts_with("GET /v1/models?limit=100 "));
    let rotated = Service::with_credential(&api, "fixture-new-key".into()).unwrap();
    assert_eq!(rotated.id(), "anthropic-api");
    assert_ne!(rotated.account(), Service::account(&api));
    let subscription = Api::new(Session::fixture("fixture-subscription-token")).unwrap();
    assert!(Service::with_credential(&subscription, key.into()).is_err());
}

#[test]
fn replacing_an_api_key_keeps_the_transport_and_uses_the_new_identity() {
    let fixture = HttpFixture::new(vec![(
        200,
        crate::json::parse(r#"{"data":[{"id":"fixture-model"}],"has_more":false}"#).unwrap(),
    )]);
    let api = api_key_client(&fixture, "fixture-original-key");
    let updated = Service::with_credential(&api, "fixture-updated-key".into()).unwrap();
    assert_eq!(updated.id(), "anthropic-api");
    assert_ne!(updated.account(), Service::account(&api));
    assert_eq!(updated.catalog().unwrap()[0].id, "fixture-model");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|line| { line.eq_ignore_ascii_case("authorization: Bearer fixture-updated-key") })
    );
    assert!(
        !requests[0]
            .headers
            .iter()
            .any(|line| line.contains("fixture-original-key"))
    );
}
