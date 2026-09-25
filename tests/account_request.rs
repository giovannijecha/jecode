use jecode::{
    json::{self, Value},
    providers::openai_account::{Error, Input, Request, Tool, encode_http},
};

fn request() -> Request {
    Request {
        model: "test-model".into(),
        instructions: "Follow the workspace instructions.".into(),
        input: vec![Input::User("quoted \"text\"\n🚀".into())],
        tools: vec![],
        effort: Some("high".into()),
    }
}

#[test]
fn request_escapes_text_and_preserves_continuation_order() {
    let mut request = request();
    let opaque = json::parse(
        r#"{"type":"reasoning","encrypted_content":"synthetic"}"#,
        Default::default(),
    )
    .unwrap();
    request.input.push(Input::Assistant(vec![opaque.clone()]));
    request.input.push(Input::ToolResult {
        call_id: "call_1".into(),
        output: "untrusted\n\"output\"".into(),
    });
    request.tools.push(Tool {
        name: "read_file".into(),
        description: "Read a file".into(),
        parameters: json::object([("type", Value::String("object".into()))]),
    });
    let body = json::parse(&request.encode(8192).unwrap(), Default::default()).unwrap();
    assert_eq!(body.get("stream"), Some(&Value::Bool(true)));
    assert_eq!(body.get("store"), Some(&Value::Bool(false)));
    assert!(body.get("text").is_none()); // No hidden verbosity/quality override.
    assert!(body.get("authorization").is_none());
    let input = body.get("input").unwrap().array().unwrap();
    assert_eq!(input.len(), 3);
    assert_eq!(
        input[0].get("content").unwrap().array().unwrap()[0]
            .get("text")
            .unwrap()
            .text(),
        Some("quoted \"text\"\n🚀")
    );
    assert_eq!(input[1], opaque);
    assert_eq!(input[2].get("call_id").unwrap().text(), Some("call_1"));
    assert_eq!(
        input[2].get("output").unwrap().text(),
        Some("untrusted\n\"output\"")
    );
}

#[test]
fn malformed_requests_and_oversized_encoding_are_rejected() {
    assert!(request().encode(1).is_err());
    let mut invalid = request();
    invalid.model = "bad\nmodel".into();
    assert_eq!(invalid.encode(8192), Err(Error::InvalidRequest));
    let mut invalid = request();
    invalid.input.push(Input::Assistant(vec![Value::Null]));
    assert_eq!(invalid.encode(8192), Err(Error::InvalidRequest));
    let mut invalid = request();
    invalid.input.push(Input::ToolResult {
        call_id: "".into(),
        output: "".into(),
    });
    assert_eq!(invalid.encode(8192), Err(Error::InvalidRequest));
    let mut invalid = request();
    for _ in 0..2 {
        invalid.tools.push(Tool {
            name: "same".into(),
            description: "".into(),
            parameters: json::object([]),
        });
    }
    assert_eq!(invalid.encode(8192), Err(Error::InvalidRequest));
}

#[test]
fn omitted_effort_is_distinct_from_literal_none() {
    let mut request = request();
    request.effort = None;
    let body = json::parse(&request.encode(8192).unwrap(), Default::default()).unwrap();
    assert!(body.get("reasoning").unwrap().get("effort").is_none());
    request.effort = Some("none".into());
    let body = json::parse(&request.encode(8192).unwrap(), Default::default()).unwrap();
    assert_eq!(
        body.get("reasoning")
            .unwrap()
            .get("effort")
            .and_then(Value::text),
        Some("none")
    );
}

#[test]
fn account_http_encodes_a_paired_multimodal_tool_result() {
    let mut request = request();
    let call = json::parse(r#"{"type":"function_call","call_id":"view_1","name":"view_image","arguments":"{\"path\":\"screen.png\"}"}"#, Default::default()).unwrap();
    request.input.push(Input::Assistant(vec![call]));
    request.input.push(Input::ToolImage {
        call_id: "view_1".into(),
        description: "PNG 1x1, captured from screen.png".into(),
        image_url: "data:image/png;base64,iVBORw0KGgo=".into(),
    });
    let wire = encode_http(&request, "synthetic-access", "synthetic-account").unwrap();
    let wire = String::from_utf8(wire).unwrap();
    assert!(wire.starts_with("POST /backend-api/codex/responses HTTP/1.1\r\n"));
    let body = wire.split_once("\r\n\r\n").unwrap().1;
    let body = json::parse(body, Default::default()).unwrap();
    let result = body
        .get("input")
        .and_then(Value::array)
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(
        result.get("type").and_then(Value::text),
        Some("function_call_output")
    );
    assert_eq!(result.get("call_id").and_then(Value::text), Some("view_1"));
    let items = result.get("output").and_then(Value::array).unwrap();
    assert_eq!(
        items[0].get("type").and_then(Value::text),
        Some("input_text")
    );
    assert_eq!(
        items[1].get("type").and_then(Value::text),
        Some("input_image")
    );
    assert_eq!(
        items[1].get("image_url").and_then(Value::text),
        Some("data:image/png;base64,iVBORw0KGgo=")
    );
    assert_eq!(items[1].get("detail").and_then(Value::text), Some("high"));
}
