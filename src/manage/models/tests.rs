use super::*;
use crate::test_support::{Directory, HttpFixture};
use std::fs;

#[test]
fn endpoint_listing_uses_tui_compatibility_without_a_model_request() {
    let fixture = HttpFixture::new(vec![(
        200,
        crate::json::parse(
            r#"{"data":{"endpoints":[
                {"tag":"tools-and-reasoning","provider_name":"First","supported_parameters":["tools","reasoning"]},
                {"tag":"tools-only","provider_name":"Second","supported_parameters":["tools"]},
                {"tag":"no-tools","provider_name":"Third","supported_parameters":["reasoning"]}
            ]}}"#,
        )
        .unwrap(),
    )]);
    let services = Services::fixture(&fixture.endpoint);
    let report = endpoints(&services, "fixture/model").unwrap();
    let rows = report
        .value
        .get("endpoints")
        .and_then(Value::as_array)
        .unwrap();
    let row = |id: &str| {
        rows.iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
            .unwrap()
    };
    assert_eq!(
        row("tools-and-reasoning").get("compatible"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        row("tools-and-reasoning").get("reasoning_compatible"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        row("tools-only").get("compatible"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        row("tools-only").get("reasoning_compatible"),
        Some(&Value::Bool(false))
    );
    assert_eq!(row("no-tools").get("compatible"), Some(&Value::Bool(false)));
    assert_eq!(
        row("no-tools").get("reasoning_compatible"),
        Some(&Value::Bool(false))
    );
    assert!(report.text.contains("--routing auto"));
    assert!(report.text.contains("--routing ENDPOINT_ID"));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].headers[0].starts_with("GET /models/fixture/model/endpoints "));
}

#[test]
fn incompatible_provider_fails_before_credentials_or_configuration_are_needed() {
    let directory = Directory::new();
    let context = Context::at(directory.path());
    let command = Manage {
        family: "models",
        action: "endpoints",
        operands: vec!["fixture/model".into()],
        values: vec![("provider", Some("openai-api".into()))],
    };
    match run(&context, &command) {
        Err(Failure::Usage(error)) => assert!(error.contains("only for openrouter"), "{error}"),
        _ => panic!("OpenAI API should not offer OpenRouter endpoint routing"),
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}
