use super::*;
use crate::session::{Model, tool_tests};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}

fn read_step(count: usize) -> Step {
    Step {
        response: Some(tool_tests::calls_response(
            (0..count)
                .map(|n| {
                    tool_tests::call(
                        &format!("saved-{n}"),
                        "read_file",
                        r#"{"path":"source.txt"}"#,
                    )
                })
                .collect(),
        )),
        results: (0..count)
            .map(|n| Receipt {
                call_id: format!("saved-{n}"),
                output: format!("original-{n}"),
                summary: "read_file / saved".into(),
                image: None,
            })
            .collect(),
        accepted: true,
        ..Default::default()
    }
}

#[test]
fn astra_followup_cached_step_crosses_256_without_repeating_cursor() {
    let fixture = crate::state::tests::Fixture::new();
    let store = fixture.store().unwrap();
    let mut history =
        crate::session::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    history.begin("Index one large call batch".into()).unwrap();
    history.turns[0].steps.push(read_step(300));
    history.checkpoint().unwrap();
    history.projection.step = 1;
    history.projection.summary = "Read source evidence".into();
    history.checkpoint().unwrap();
    history.release_projected();
    let cancelled = AtomicBool::new(false);
    let mut position = 0;
    let mut seen = Vec::new();
    loop {
        let output = index(&history, 0, 0, position, &budget(&cancelled));
        assert!(!output.failed, "{}", output.text);
        let value = json::parse(&output.text, Default::default()).unwrap();
        for entry in value.get("entries").and_then(Value::array).unwrap() {
            seen.push(
                entry
                    .get("recall_address")
                    .unwrap()
                    .get("receipt")
                    .and_then(Value::unsigned)
                    .unwrap() as usize,
            );
        }
        let next = value.get("next").unwrap();
        if *next == Value::Null {
            break;
        }
        let next_position = next.get("receipt").and_then(Value::unsigned).unwrap() as usize;
        assert!(
            next_position > position,
            "index stopped progressing after {} receipts; current={position}; next={next_position}; output={}",
            seen.len(),
            output.text
        );
        position = next_position;
    }
    assert_eq!(seen, (0..300).collect::<Vec<_>>());
}

#[test]
fn astra_followup_resident_failed_step_does_not_hide_later_receipts() {
    let mut history = History::default();
    history
        .begin("A failed request followed by completed reads".into())
        .unwrap();
    history.turns[0].steps.push(Step::default());
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(Vec::new())),
        ..Default::default()
    });
    let mut incomplete = tool_tests::calls_response(Vec::new());
    incomplete.status = Status::Incomplete;
    history.turns[0].steps.push(Step {
        response: Some(incomplete),
        accepted: true,
        ..Default::default()
    });
    history.turns[0].steps.push(read_step(1));
    history.projection.step = 4;
    history.projection.summary = "Earlier failed request and saved read".into();
    let cancelled = AtomicBool::new(false);
    let short_budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_millis(100),
    };
    let output = index(&history, 0, 0, 0, &short_budget);
    assert!(
        !output.failed && output.text.contains("saved-0"),
        "failed step blocks later evidence: {}",
        output.text
    );
    let page = json::parse(&output.text, Default::default()).unwrap();
    let entries = page.get("entries").and_then(Value::array).unwrap();
    assert_eq!(entries.len(), 1);
    let address = entries[0].get("recall_address").unwrap();
    assert_eq!(address.get("step").and_then(Value::unsigned), Some(3));
    assert_eq!(address.get("receipt").and_then(Value::unsigned), Some(0));
}

fn mixed_read_step() -> (Step, Vec<usize>) {
    let mut calls = Vec::new();
    let mut results = Vec::new();
    let mut expected = Vec::new();
    for receipt in 0..300 {
        let outside_scope = receipt != 0 && receipt % 5 == 0;
        let unexecuted = receipt != 0 && receipt % 7 == 0;
        calls.push(tool_tests::call(
            &format!("mixed-{receipt}"),
            if outside_scope {
                "run_command"
            } else {
                "read_file"
            },
            r#"{"path":"source.txt"}"#,
        ));
        results.push(Receipt {
            call_id: format!("mixed-{receipt}"),
            output: format!("original-{receipt}"),
            summary: if unexecuted {
                "Not executed"
            } else {
                "read_file / saved"
            }
            .into(),
            image: None,
        });
        if !outside_scope && !unexecuted {
            expected.push(receipt);
        }
    }
    (
        Step {
            response: Some(tool_tests::calls_response(calls)),
            results,
            accepted: true,
            ..Default::default()
        },
        expected,
    )
}

fn mixed_pagination(oversized_metadata: bool) {
    let fixture = crate::state::tests::Fixture::new();
    let store = fixture.store().unwrap();
    let mut history =
        crate::session::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("Index mixed saved calls".into()).unwrap();
    let (step, expected) = mixed_read_step();
    history.turns[0].steps.push(step);
    if oversized_metadata {
        // These ordinary saved steps exceed the turn metadata budget, forcing
        // 256-identity windows while keeping the whole turn retrievable.
        for _ in 0..10 {
            history.turns[0].steps.push(Step {
                text: "x".repeat(900_000),
                ..Default::default()
            });
        }
    }
    history.checkpoint().unwrap();
    history.projection.step = history.turns[0].steps.len();
    history.projection.summary = "Saved reads".into();
    history.checkpoint().unwrap();
    history.release_projected();
    let cancelled = AtomicBool::new(false);
    let long_budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(120),
    };
    let first = history
        .record
        .as_ref()
        .unwrap()
        .indexed_step(0, 0, 0, &long_budget)
        .unwrap()
        .unwrap();
    assert_eq!(first.receipt_base, 0);
    assert_eq!(
        first.receipts.len(),
        if oversized_metadata { 256 } else { 300 }
    );
    let mut cursor = (0, 0, 0);
    let mut seen = Vec::new();
    let mut pages = 0;
    loop {
        let output = index(&history, cursor.0, cursor.1, cursor.2, &long_budget);
        assert!(!output.failed, "{}", output.text);
        assert!(output.text.len() <= 8 * 1024);
        let page = json::parse(&output.text, Default::default()).unwrap();
        for entry in page.get("entries").and_then(Value::array).unwrap() {
            let address = entry.get("recall_address").unwrap();
            let receipt = address.get("receipt").and_then(Value::unsigned).unwrap() as usize;
            assert_eq!(address.get("turn").and_then(Value::unsigned), Some(0));
            assert_eq!(address.get("step").and_then(Value::unsigned), Some(0));
            assert_eq!(
                address.get("expected_call_id").and_then(Value::text),
                Some(format!("mixed-{receipt}").as_str())
            );
            assert_eq!(
                entry
                    .get("arguments")
                    .and_then(|args| args.get("path"))
                    .and_then(Value::text),
                Some("source.txt")
            );
            seen.push(receipt);
        }
        pages += 1;
        let next = page.get("next").unwrap();
        if *next == Value::Null {
            break;
        }
        let next_cursor = (
            next.get("turn").and_then(Value::unsigned).unwrap() as usize,
            next.get("step").and_then(Value::unsigned).unwrap() as usize,
            next.get("receipt").and_then(Value::unsigned).unwrap() as usize,
        );
        assert!(next_cursor > cursor, "{}", output.text);
        cursor = next_cursor;
        if pages == 2 {
            assert_eq!(cursor.1, 0);
            drop(history);
            history = crate::session::persistence::load(&store, &id, true)
                .unwrap()
                .history;
            let resumed = history
                .record
                .as_ref()
                .unwrap()
                .indexed_step(0, 0, cursor.2, &long_budget)
                .unwrap()
                .unwrap();
            assert_eq!(
                resumed.receipt_base,
                if oversized_metadata { cursor.2 } else { 0 }
            );
        }
    }
    assert!(pages > 2);
    assert_eq!(seen, expected);
    let boundary = history
        .record
        .as_ref()
        .unwrap()
        .indexed_step(0, 0, 256, &long_budget)
        .unwrap()
        .unwrap();
    assert_eq!(
        boundary.receipt_base,
        if oversized_metadata { 256 } else { 0 }
    );
    assert!(seen.contains(&0) && seen.contains(&256) && seen.contains(&299));
    assert!(!seen.contains(&255) && !seen.contains(&259));
    for receipt in [0, 256, 299] {
        let guard = format!("mixed-{receipt}");
        let output = execute_with_identity(&history, 0, 0, receipt, 0, Some(&guard), &long_budget);
        assert!(!output.failed, "{}", output.text);
        assert!(output.text.contains(&format!("original-{receipt}")));
    }
    assert!(execute_with_identity(&history, 0, 0, 256, 0, Some("wrong"), &long_budget).failed);
}

#[test]
fn complete_cache_pages_mixed_receipts_after_close_resume() {
    mixed_pagination(false);
}

#[test]
fn oversized_metadata_windows_page_mixed_receipts_after_close_resume() {
    mixed_pagination(true);
}
