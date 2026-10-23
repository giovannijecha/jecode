use super::*;
use crate::test_support::{HttpFixture, completion};

#[test]
fn output_allowance_respects_both_model_output_and_remaining_context() {
    for (output, input, expected) in [
        (Some(9000), 1000, 9000),
        (Some(9000), 20000, 4000),
        (Some(32000), 1000, 23000),
        (None, 20000, 4000),
        (Some(9000), 25000, 0),
    ] {
        assert_eq!(
            Limits {
                context: 24000,
                output
            }
            .output_allowance(input),
            expected
        );
    }
}

#[test]
fn summary_requests_use_plain_text_regardless_of_model_json_support() {
    for supported in [false, true] {
        let parameters = if supported {
            vec![Value::string("response_format")]
        } else {
            vec![]
        };
        let catalog = Value::object([(
            "data",
            Value::Array(vec![
                Value::object([
                    ("id", Value::string("fixture/other")),
                    (
                        "supported_parameters",
                        Value::Array(vec![Value::string("response_format")]),
                    ),
                ]),
                Value::object([
                    ("id", Value::string("fixture/model")),
                    ("context_length", Value::number(24000)),
                    ("supported_parameters", Value::Array(parameters)),
                ]),
            ]),
        )]);
        let fixture = HttpFixture::new(vec![
            (200, catalog),
            (200, completion("Handoff summary", vec![])),
            (200, completion("Done", vec![])),
            (200, completion("Another summary", vec![])),
        ]);
        let mut client = OpenRouter::fixture(fixture.endpoint.clone());
        client.limits = None;
        client.limits().unwrap();
        let messages = [Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Summarize")),
        ])];
        client
            .sample(&messages, false, Some(3000), &mut |_| Ok(()))
            .unwrap();
        client
            .sample(&messages, true, None, &mut |_| Ok(()))
            .unwrap();
        client.set_model("fixture/new-model".into()).unwrap();
        client
            .sample(&messages, false, Some(3000), &mut |_| Ok(()))
            .unwrap();
        let requests = fixture.finish();
        assert_eq!(requests.len(), 4);
        assert!(requests[1].body.get("response_format").is_none());
        assert!(requests[1].body.get("tools").is_none());
        assert!(requests[2].body.get("response_format").is_none());
        assert!(requests[2].body.get("tools").is_some());
        assert!(requests[3].body.get("response_format").is_none());
    }
}

#[test]
fn pinned_endpoints_do_not_add_json_output_to_summary_requests() {
    let catalog = Value::object([(
        "data",
        Value::Array(vec![Value::object([
            ("id", Value::string("fixture/model")),
            ("context_length", Value::number(24000)),
            (
                "supported_parameters",
                Value::Array(vec![Value::string("response_format")]),
            ),
        ])]),
    )]);
    let mut responses = vec![(200, catalog)];
    responses.extend((0..3).map(|_| (200, completion("Handoff summary", vec![]))));
    let fixture = HttpFixture::new(responses);
    let mut client = OpenRouter::fixture(fixture.endpoint.clone());
    client.limits = None;
    client.limits().unwrap();
    let listing = crate::json::parse(
        r#"{"data":{"endpoints":[{"tag":"plain/one","supported_parameters":["tools"]},
        {"tag":"json/two","supported_parameters":["tools","response_format"]}]}}"#,
    )
    .unwrap();
    client.offer_endpoints(
        crate::backend::openrouter::endpoints::parse(
            "fixture/model",
            &listing,
            std::time::Instant::now(),
        )
        .unwrap(),
    );
    let messages = [Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Summarize")),
    ])];
    for tag in ["plain/one", "json/two", "unlisted/three"] {
        client.set_routing(crate::backend::openrouter::Routing::Endpoint(tag.into()));
        client
            .sample(&messages, false, Some(3000), &mut |_| Ok(()))
            .unwrap();
    }
    let requests = fixture.finish();
    let formats: Vec<_> = requests[1..]
        .iter()
        .map(|request| request.body.get("response_format").is_some())
        .collect();
    assert_eq!(formats, [false, false, false]);
}
