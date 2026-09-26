use super::*;
use crate::{
    json::{self, Value},
    session::{
        End,
        history::{Receipt, Step},
        receipt_recall, tool_tests,
    },
    tools::Prepared,
};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn read(id: &str, output: &str) -> Receipt {
    Receipt {
        call_id: id.into(),
        output: output.into(),
        summary: format!("read_file / {id}.txt"),
        image: None,
    }
}

fn address(reference: &partial::Reference) -> Value {
    reference
        .association
        .split_once("; recall_address=")
        .map_or(Value::Null, |(_, address)| {
            json::parse(address, Default::default()).unwrap()
        })
}

fn retrieve(history: &History, address: &Value) -> crate::tools::Output {
    let Prepared::Recall {
        turn,
        step,
        receipt,
        offset,
        expected_call_id,
    } = Prepared::parse("recall_receipts", address).unwrap()
    else {
        unreachable!()
    };
    let cancelled = AtomicBool::new(false);
    receipt_recall::execute_with_identity(
        history,
        turn,
        step,
        receipt,
        offset,
        expected_call_id.as_deref(),
        &crate::workspace::Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(10),
        },
    )
}

#[test]
fn emitted_addresses_select_first_middle_and_last_with_interleaved_output() {
    let mut history = History::default();
    history.begin("Recover three exact reads".into()).unwrap();
    let mut response = tool_tests::calls_response(vec![
        tool_tests::call("read-first", "read_file", r#"{"path":"first.txt"}"#),
        tool_tests::call("read-middle", "read_file", r#"{"path":"middle.txt"}"#),
        tool_tests::call("read-last", "read_file", r#"{"path":"last.txt"}"#),
    ]);
    response
        .output
        .retain(|item| item.get("type").and_then(Value::text) == Some("function_call"));
    response
        .output
        .insert(0, json::object([("type", Value::String("message".into()))]));
    response.output.insert(
        2,
        json::object([("type", Value::String("reasoning".into()))]),
    );
    response
        .output
        .insert(4, json::object([("type", Value::String("message".into()))]));
    history.turns[0].steps.push(Step {
        response: Some(response),
        results: vec![
            read("read-first", "FIRST-EXACT"),
            read("read-middle", "MIDDLE-EXACT"),
            read("read-last", "LAST-EXACT"),
        ],
        accepted: true,
        ..Default::default()
    });
    let definitions = crate::tools::definitions();
    let schema = definitions
        .iter()
        .find(|tool| tool.name == "recall_receipts")
        .unwrap();
    let properties = schema.parameters.get("properties").unwrap();
    assert!(properties.get("expected_call_id").is_some());
    assert!(
        !schema
            .parameters
            .get("required")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .any(|field| field.text() == Some("expected_call_id"))
    );
    for (record, output_index, receipt, id, exact) in [
        (2, 1, 0, "read-first", "FIRST-EXACT"),
        (4, 3, 1, "read-middle", "MIDDLE-EXACT"),
        (6, 5, 2, "read-last", "LAST-EXACT"),
    ] {
        let reference = partial::reference_at_position(&history, 0, 0, record).unwrap();
        let value = json::parse(&reference.content, Default::default()).unwrap();
        assert_eq!(
            value.get("output_index").and_then(Value::unsigned),
            Some(output_index)
        );
        assert!(value.get("tool_call_index").is_none());
        assert!(value.get("recall_address").is_none());
        let address = address(&reference);
        assert_eq!(address.get("turn").and_then(Value::unsigned), Some(0));
        assert_eq!(address.get("step").and_then(Value::unsigned), Some(0));
        assert_eq!(
            address.get("receipt").and_then(Value::unsigned),
            Some(receipt)
        );
        assert_eq!(
            address.get("expected_call_id").and_then(Value::text),
            Some(id)
        );
        assert!(reference.association.contains("recall_address="));
        let page = retrieve(&history, &address);
        assert!(!page.failed, "{}", page.text);
        let page = json::parse(&page.text, Default::default()).unwrap();
        assert_eq!(page.get("output").and_then(Value::text), Some(exact));
        assert_eq!(page.get("call_id").and_then(Value::text), Some(id));
    }
    let mut wrong = address(&partial::reference_at_position(&history, 0, 0, 2).unwrap());
    if let Value::Object(fields) = &mut wrong {
        fields.insert("receipt".into(), Value::Number("1".into()));
    }
    let denied = retrieve(&history, &wrong);
    assert!(denied.failed);
    assert!(denied.text.contains("expected_call_id does not match"));
    assert!(!denied.text.contains("MIDDLE-EXACT"));
    if let Value::Object(fields) = &mut wrong {
        fields.remove("expected_call_id");
    }
    let historical = retrieve(&history, &wrong);
    assert!(!historical.failed);
    assert!(historical.text.contains("MIDDLE-EXACT"));
    for invalid in [
        Value::Null,
        Value::String(String::new()),
        Value::String("x".repeat(257)),
    ] {
        let mut args = wrong.clone();
        if let Value::Object(fields) = &mut args {
            fields.insert("expected_call_id".into(), invalid);
        }
        assert!(Prepared::parse("recall_receipts", &args).is_err());
    }
}

#[test]
fn mixed_results_advertise_only_eligible_original_indices() {
    let mut history = History::default();
    history.begin("Inspect mixed calls".into()).unwrap();
    let calls = vec![
        tool_tests::call("first", "read_file", r#"{"path":"a.txt"}"#),
        tool_tests::call("effect", "run_command", r#"{"command":"echo x"}"#),
        tool_tests::call("unexecuted", "read_file", r#"{"path":"b.txt"}"#),
        tool_tests::call("uncertain", "search_text", r#"{"query":"x"}"#),
        tool_tests::call("mismatch", "read_file", r#"{"path":"c.txt"}"#),
        tool_tests::call("last", "read_file", r#"{"path":"d.txt"}"#),
    ];
    let mut response = tool_tests::calls_response(calls);
    response
        .output
        .retain(|item| item.get("type").and_then(Value::text) == Some("function_call"));
    history.turns[0].steps.push(Step {
        response: Some(response),
        results: vec![
            read("first", "FIRST"),
            read("effect", "PRIVATE EFFECT"),
            read(
                "unexecuted",
                &crate::tools::Output::not_executed("cancelled").text,
            ),
            read("uncertain", r#"{"ok":false,"status":"uncertain"}"#),
            read("wrong-call", "UNPAIRED"),
            read("last", "LAST"),
        ],
        accepted: true,
        ..Default::default()
    });
    for receipt in 1..=4 {
        let reference = partial::reference_at_position(&history, 0, 0, receipt + 1).unwrap();
        assert_eq!(address(&reference), Value::Null);
        assert!(!reference.association.contains("recall_address="));
    }
    let last = address(&partial::reference_at_position(&history, 0, 0, 6).unwrap());
    assert_eq!(last.get("receipt").and_then(Value::unsigned), Some(5));
    assert_eq!(
        last.get("expected_call_id").and_then(Value::text),
        Some("last")
    );
    let first = address(&partial::reference_at_position(&history, 0, 0, 1).unwrap());
    let first_page = retrieve(&history, &first);
    let value = json::parse(&first_page.text, Default::default()).unwrap();
    assert_eq!(value.get("next"), Some(&last));
    assert_eq!(
        json::parse(&retrieve(&history, &last).text, Default::default())
            .unwrap()
            .get("output")
            .and_then(Value::text),
        Some("LAST")
    );
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn absolute_addresses_survive_released_prefixes_resume_and_source_deletion() {
    let home = crate::state::tests::Fixture::new();
    let store = home.store().unwrap();
    let files = crate::workspace_fixture::Fixture::new();
    let workspace = crate::workspace::Workspace::open(&files.0).unwrap();
    let mut history =
        crate::session::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let session_id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("Read three sources".into()).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = crate::workspace::Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let mut exact = Vec::new();
    for (index, content) in ["FIRST-é", "MIDDLE-€", "LAST-Ω"].iter().enumerate() {
        let path = format!("source-{index}.txt");
        let call_id = format!("read-{index}");
        files.write(&path, content);
        let arguments = json::object([("path", Value::String(path.clone()))]);
        let output = Prepared::parse("read_file", &arguments)
            .unwrap()
            .execute(&workspace, &budget);
        assert!(!output.failed);
        exact.push(output.text.clone());
        let mut response = tool_tests::calls_response(vec![tool_tests::call(
            &call_id,
            "read_file",
            &json::encode(&arguments, 4096).unwrap(),
        )]);
        response
            .output
            .retain(|item| item.get("type").and_then(Value::text) == Some("function_call"));
        history.turns[0].steps.push(Step {
            response: Some(response),
            results: vec![read(&call_id, &output.text)],
            accepted: true,
            ..Default::default()
        });
    }
    history.turns[0].end = Some(End::Complete);
    history.checkpoint().unwrap();
    let first = address(&partial::reference_at_position(&history, 0, 0, 1).unwrap());
    history.projection.step = 1;
    history.projection.summary = "First read compacted".into();
    history.checkpoint().unwrap();
    history.release_projected();
    assert_eq!(history.base_step, 1);
    let middle = address(&partial::reference_at_position(&history, 0, 0, 1).unwrap());
    history.projection.step = 1;
    history.projection.summary = "First and middle reads compacted".into();
    history.checkpoint().unwrap();
    history.release_projected();
    assert_eq!(history.base_step, 2);
    let last = address(&partial::reference_at_position(&history, 0, 0, 1).unwrap());
    for (step, reference) in [&first, &middle, &last].iter().enumerate() {
        assert_eq!(reference.get("turn").and_then(Value::unsigned), Some(0));
        assert_eq!(
            reference.get("step").and_then(Value::unsigned),
            Some(step as u64)
        );
        assert_eq!(reference.get("receipt").and_then(Value::unsigned), Some(0));
    }
    drop(history);
    for index in 0..3 {
        std::fs::remove_file(files.0.join(format!("source-{index}.txt"))).unwrap();
    }
    let saved = crate::session::persistence::load(&store, &session_id, true).unwrap();
    for (index, reference) in [first, middle, last].iter().enumerate() {
        let page = retrieve(&saved.history, reference);
        assert!(!page.failed, "{}", page.text);
        let value = json::parse(&page.text, Default::default()).unwrap();
        assert_eq!(
            value.get("output").and_then(Value::text),
            Some(exact[index].as_str())
        );
        assert_eq!(
            value.get("call_id").and_then(Value::text),
            Some(format!("read-{index}").as_str())
        );
    }
}
