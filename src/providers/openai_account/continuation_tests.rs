use super::{Error, Input, Limits, Request, Response, ResponseStream};
use crate::json::{self, Value};
use std::ops::ControlFlow;

fn decoded(signal: Option<Value>) -> Result<Response, Error> {
    let message = json::object([
        ("type", Value::String("message".into())),
        ("id", Value::String("message-1".into())),
        ("role", Value::String("assistant".into())),
        ("phase", Value::String("commentary".into())),
        ("status", Value::String("completed".into())),
        (
            "content",
            Value::Array(vec![json::object([
                ("type", Value::String("output_text".into())),
                ("text", Value::String("Working on it.".into())),
            ])]),
        ),
    ]);
    let item = json::object([
        ("type", Value::String("response.output_item.done".into())),
        ("output_index", Value::Number("0".into())),
        ("item", message),
    ]);
    let mut response = vec![
        ("id", Value::String("response-1".into())),
        ("status", Value::String("completed".into())),
        ("output", Value::Array(Vec::new())),
    ];
    if let Some(signal) = signal {
        response.push(("end_turn", signal));
    }
    let terminal = json::object([
        ("type", Value::String("response.completed".into())),
        ("response", json::object(response)),
    ]);
    let wire = format!(
        "data: {}\n\ndata: {}\n\n",
        json::encode(&item, 4096).unwrap(),
        json::encode(&terminal, 4096).unwrap()
    );
    let mut stream = ResponseStream::new(Limits::default());
    for chunk in wire.as_bytes().chunks(17) {
        stream.push(chunk, |_| ControlFlow::Continue(()))?;
    }
    stream.finish()
}

#[test]
fn optional_end_turn_distinguishes_explicit_false_true_and_absence() {
    for (signal, expected) in [
        (None, None),
        (Some(Value::Null), None),
        (Some(Value::Bool(true)), Some(true)),
        (Some(Value::Bool(false)), Some(false)),
    ] {
        let response = decoded(signal).unwrap();
        assert_eq!(response.end_turn, expected);
        let snapshot = response.snapshot();
        assert_eq!(snapshot.get("end_turn").cloned(), expected.map(Value::Bool));
        assert_eq!(Response::restore(&snapshot).unwrap(), response);
    }
    for malformed in [
        Value::String("false".into()),
        Value::Number("0".into()),
        Value::Array(Vec::new()),
    ] {
        assert_eq!(decoded(Some(malformed)), Err(Error::InvalidEvent));
    }
    let mut corrupt = decoded(Some(Value::Bool(false))).unwrap().snapshot();
    if let Value::Object(fields) = &mut corrupt {
        fields.insert("end_turn".into(), Value::String("false".into()));
    }
    assert_eq!(Response::restore(&corrupt), Err(Error::InvalidEvent));
}

#[test]
fn streamed_message_phase_survives_snapshot_and_next_request() {
    let response = decoded(Some(Value::Bool(false))).unwrap();
    assert_eq!(
        response.output[0].get("phase").and_then(Value::text),
        Some("commentary")
    );
    let restored = Response::restore(&response.snapshot()).unwrap();
    let request = Request {
        model: "gpt-6-luna".into(),
        instructions: "fixture".into(),
        input: vec![Input::Assistant(restored.output)],
        tools: Vec::new(),
        effort: Some("medium".into()),
    };
    let body = json::parse(&request.encode(8192).unwrap(), Default::default()).unwrap();
    let input = body.get("input").and_then(Value::array).unwrap();
    assert_eq!(
        input[0].get("phase").and_then(Value::text),
        Some("commentary")
    );
    assert_eq!(input[0].get("id").and_then(Value::text), Some("message-1"));
}
