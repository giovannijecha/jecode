use super::{Backend, openai, openrouter};
use crate::{
    auth,
    json::Value,
    test_support::{HttpFixture, Response},
};
use std::time::Duration;

fn response() -> Response {
    let event = Value::object([
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
                                ("text", Value::string("Complete.")),
                            ])]),
                        ),
                    ])]),
                ),
            ]),
        ),
    ]);
    Response::Stream(vec![(
        Duration::ZERO,
        format!("data: {}\n\n", event.encode()),
    )])
}

#[test]
fn both_native_backends_omit_delegation_from_child_requests() {
    for provider in ["openrouter", "openai"] {
        let fixture = if provider == "openai" {
            HttpFixture::streaming(vec![response(), response()])
        } else {
            let reply = crate::test_support::completion("Complete.", vec![]);
            HttpFixture::new(vec![(200, reply.clone()), (200, reply)])
        };
        let mut client: Box<dyn Backend> = if provider == "openai" {
            Box::new(
                openai::OpenAi::with_api(
                    openai::Api::fixture(
                        auth::openai::Session::fixture("isolated-child-token"),
                        fixture.endpoint.trim_end_matches("/chat/completions"),
                    ),
                    "fixture-model".into(),
                )
                .unwrap(),
            )
        } else {
            Box::new(
                openrouter::OpenRouter::with_api(
                    openrouter::Api::fixture(&fixture.endpoint),
                    "fixture/model".into(),
                )
                .unwrap(),
            )
        };
        for enabled in [false, true] {
            client.set_delegation(enabled);
            client
                .sample(
                    &[Value::object([
                        ("role", Value::string("user")),
                        ("content", Value::string("Inspect the assigned work.")),
                    ])],
                    true,
                    None,
                    None,
                    &mut |_| Ok(()),
                )
                .unwrap();
        }
        let requests = fixture.finish();
        for (request, enabled) in requests.iter().zip([false, true]) {
            let outer = request.body.get("tools").and_then(Value::as_array).unwrap();
            let definitions = if provider == "openai" {
                outer[0]
                    .get("tools")
                    .and_then(Value::as_array)
                    .unwrap()
                    .to_vec()
            } else {
                outer
                    .iter()
                    .map(|t| t.get("function").unwrap().clone())
                    .collect()
            };
            let names: Vec<_> = definitions
                .iter()
                .map(|t| t.get("name").and_then(Value::as_str).unwrap())
                .collect();
            let expected: Vec<_> = crate::tools::definitions_for(enabled)
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t.get("name").and_then(Value::as_str).unwrap())
                .collect();
            assert_eq!(names, expected, "{provider}, delegation={enabled}");
            assert_eq!(names.contains(&"agent"), enabled);
            for working_tool in ["read", "write", "edit", "bash", "exec", "browser", "map"] {
                assert!(
                    names.contains(&working_tool),
                    "{provider}: missing {working_tool}"
                );
            }
        }
    }
}
