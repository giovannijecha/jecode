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
fn summary_json_is_negotiated_for_the_exact_model_and_never_applied_to_tool_turns() {
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
            (200, completion("{}", vec![])),
            (200, completion("Done", vec![])),
            (200, completion("{}", vec![])),
        ]);
        let mut client = OpenRouter::fixture(fixture.endpoint.clone());
        client.limits = None;
        client.limits().unwrap();
        let messages = [Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("JSON")),
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
        let format = requests[1].body.get("response_format");
        assert_eq!(format.is_some(), supported);
        if supported {
            assert_eq!(
                format.unwrap().get("type").and_then(Value::as_str),
                Some("json_object")
            );
        }
        assert!(requests[1].body.get("tools").is_none());
        assert!(requests[2].body.get("response_format").is_none());
        assert!(requests[2].body.get("tools").is_some());
        assert!(requests[3].body.get("response_format").is_none());
    }
}
