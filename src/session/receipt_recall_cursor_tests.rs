use super::*;
use crate::session::{End, Model, history::Receipt, tool_tests};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn utf8_cursor_skips_effect_and_unexecuted_sibling_after_resume() {
    let fixture = crate::state::tests::Fixture::new();
    let store = fixture.store().unwrap();
    let mut history =
        super::super::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    let first = "α€".repeat(2500);
    let last = "LAST-READ-Ω";
    history.begin("Inspect original reads".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![
            tool_tests::call("read-a", "read_file", r#"{"path":"a.txt"}"#),
            tool_tests::call(
                "edit-a",
                "edit_file",
                r#"{"path":"a.txt","old_text":"a","new_text":"b"}"#,
            ),
            tool_tests::call("read-cancelled", "read_file", r#"{"path":"cancelled.txt"}"#),
            tool_tests::call("read-last", "read_file", r#"{"path":"last.txt"}"#),
        ])),
        results: vec![
            Receipt {
                call_id: "read-a".into(),
                output: first.clone(),
                summary: "read_file / a.txt".into(),
                image: None,
            },
            Receipt {
                call_id: "edit-a".into(),
                output: "private effect output".into(),
                summary: "edit_file / applied".into(),
                image: None,
            },
            Receipt {
                call_id: "read-cancelled".into(),
                output: crate::tools::Output::not_executed("later sibling was cancelled").text,
                summary: "read_file / later sibling was cancelled".into(),
                image: None,
            },
            Receipt {
                call_id: "read-last".into(),
                output: last.into(),
                summary: "read_file / last.txt".into(),
                image: None,
            },
        ],
        accepted: true,
        ..Default::default()
    });
    history.turns[0].end = Some(End::Failed(crate::session::Failure::Cancelled));
    history.checkpoint().unwrap();
    history.projection.step = 1;
    history.projection.summary = "Original read receipts remain available.".into();
    history.checkpoint().unwrap();
    history.release_projected();
    drop(history);
    let saved = super::super::persistence::load(&store, &id, true).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let mut cursor = address(0, 0, 0, 0, "read-a");
    let mut assembled = String::new();
    loop {
        let crate::tools::Prepared::Recall {
            turn,
            step,
            receipt,
            offset,
            expected_call_id,
        } = crate::tools::Prepared::parse("recall_receipts", &cursor).unwrap()
        else {
            unreachable!()
        };
        let page = execute_with_identity(
            &saved.history,
            turn,
            step,
            receipt,
            offset,
            expected_call_id.as_deref(),
            &budget,
        );
        assert!(!page.failed, "{}", page.text);
        let value = json::parse(&page.text, Default::default()).unwrap();
        assert_eq!(
            value.get("call_id").and_then(Value::text),
            Some(if receipt == 0 { "read-a" } else { "read-last" })
        );
        if receipt == 0 {
            assembled.push_str(value.get("output").and_then(Value::text).unwrap());
        } else {
            assert_eq!(value.get("output").and_then(Value::text), Some(last));
            assert!(matches!(value.get("next"), Some(Value::Null)));
            break;
        }
        cursor = value.get("next").unwrap().clone();
        let next_receipt = cursor.get("receipt").and_then(Value::unsigned).unwrap();
        assert!(next_receipt == 0 || next_receipt == 3);
        assert_eq!(
            cursor.get("expected_call_id").and_then(Value::text),
            Some(if next_receipt == 0 {
                "read-a"
            } else {
                "read-last"
            })
        );
        if next_receipt == 3 {
            assert_eq!(cursor.get("offset").and_then(Value::unsigned), Some(0));
        }
    }
    assert_eq!(assembled, first);
    for index in [1, 2] {
        let denied = execute(&saved.history, 0, 0, index, 0, &budget);
        assert!(denied.failed);
        assert!(!denied.text.contains("private effect output"));
    }
}

#[test]
fn mismatched_or_uncertain_target_is_rejected_without_hiding_valid_sibling() {
    let mut history = History::default();
    history.begin("Inspect receipts".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![
            tool_tests::call("valid", "read_file", r#"{"path":"a.txt"}"#),
            tool_tests::call("mismatch", "read_file", r#"{"path":"b.txt"}"#),
            tool_tests::call("uncertain", "read_file", r#"{"path":"c.txt"}"#),
        ])),
        results: vec![
            Receipt {
                call_id: "valid".into(),
                output: "OBSERVED".into(),
                summary: "read_file / a.txt".into(),
                image: None,
            },
            Receipt {
                call_id: "wrong-id".into(),
                output: "UNPAIRED".into(),
                summary: "read_file / b.txt".into(),
                image: None,
            },
            Receipt {
                call_id: "uncertain".into(),
                output: r#"{"ok":false,"status":"uncertain"}"#.into(),
                summary: "read_file / outcome unknown after interruption".into(),
                image: None,
            },
        ],
        accepted: true,
        ..Default::default()
    });
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let valid = execute(&history, 0, 0, 0, 0, &budget);
    assert!(!valid.failed);
    let value = json::parse(&valid.text, Default::default()).unwrap();
    assert!(matches!(value.get("next"), Some(Value::Null)));
    for index in [1, 2] {
        assert!(execute(&history, 0, 0, index, 0, &budget).failed);
    }
}
