use super::*;
use crate::{
    session::{Model, context::partial::Pending, history::Receipt, tool_tests},
    workspace::Workspace,
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn confident_error_and_omission_remain_checkable_after_source_changes_and_resume() {
    for handoff in [
        "Checked alpha=7, beta=0, total=7.",
        "Read alpha=7; beta and total omitted.",
    ] {
        let summary = json::encode(
            &json::object([
                ("source_boundary", Value::String("turn=0 step=1".into())),
                (
                    "objectives",
                    Value::Array(vec![Value::String("Report both source values".into())]),
                ),
                ("constraints", Value::Array(Vec::new())),
                ("decisions", Value::Array(Vec::new())),
                (
                    "completed",
                    Value::Array(vec![Value::String(handoff.into())]),
                ),
                ("checks", Value::Array(Vec::new())),
                ("remaining", Value::Array(Vec::new())),
                ("uncertainties", Value::Array(Vec::new())),
                (
                    "evidence",
                    Value::Array(vec![Value::String("original source reads".into())]),
                ),
            ]),
            32768,
        )
        .unwrap();
        assert_eq!(
            crate::session::context::handoff::valid(&summary, "turn=0 step=1"),
            Ok(())
        );
        let fixture = crate::state::tests::Fixture::new();
        let store = fixture.store().unwrap();
        let files = crate::workspace_fixture::Fixture::new();
        files.write("alpha.txt", "alpha=7\n");
        files.write("beta.txt", "beta=13\n");
        let workspace = Workspace::open(&files.0).unwrap();
        let cancelled = AtomicBool::new(false);
        let captured = ["alpha.txt", "beta.txt"].map(|path| {
            let args = json::object([("path", Value::String(path.into()))]);
            let output = Prepared::parse("read_file", &args)
                .unwrap()
                .execute(&workspace, &budget(&cancelled));
            assert!(!output.failed);
            output.text
        });
        let mut history =
            crate::session::persistence::create_in(&store, Model::Luna, None, None).unwrap();
        let id = history.record.as_ref().unwrap().id().to_owned();
        history
            .begin("Report both saved source values".into())
            .unwrap();
        let calls = ["alpha.txt", "beta.txt"]
            .iter()
            .enumerate()
            .map(|(n, path)| {
                tool_tests::call(
                    &format!("read-{n}"),
                    "read_file",
                    &format!(r#"{{"path":"{path}"}}"#),
                )
            })
            .collect();
        history.turns[0].steps.push(Step {
            response: Some(tool_tests::calls_response(calls)),
            results: captured
                .iter()
                .enumerate()
                .map(|(n, output)| Receipt {
                    call_id: format!("read-{n}"),
                    output: output.clone(),
                    summary: "read_file / captured".into(),
                    image: None,
                })
                .collect(),
            accepted: true,
            ..Default::default()
        });
        history.checkpoint().unwrap();
        history.projection.step = 1;
        history.projection.summary = summary;
        history.checkpoint().unwrap();
        history.release_projected();
        files.write("alpha.txt", "alpha=999\n");
        std::fs::remove_file(files.0.join("beta.txt")).unwrap();
        drop(history);

        let saved = crate::session::persistence::load(&store, &id, true).unwrap();
        let input = saved.history.input(saved.history.turns.len()).unwrap();
        assert!(input.iter().any(|item| {
            matches!(item, crate::providers::openai_account::Input::User(text)
                if text.contains(handoff) && text.contains("not checked against source receipts"))
        }));
        let listing = index(&saved.history, 0, 0, 0, &budget(&cancelled));
        assert!(!listing.failed, "{}", listing.text);
        let value = json::parse(&listing.text, Default::default()).unwrap();
        let entries = value.get("entries").and_then(Value::array).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(matches!(value.get("next"), Some(Value::Null)));
        for (n, entry) in entries.iter().enumerate() {
            let address = entry.get("recall_address").unwrap();
            let call_id = address
                .get("expected_call_id")
                .and_then(Value::text)
                .unwrap();
            assert_eq!(call_id, format!("read-{n}"));
            let recalled = execute_with_identity(
                &saved.history,
                0,
                0,
                n,
                0,
                Some(call_id),
                &budget(&cancelled),
            );
            assert!(!recalled.failed, "{}", recalled.text);
            let page = json::parse(&recalled.text, Default::default()).unwrap();
            assert_eq!(
                page.get("output").and_then(Value::text),
                Some(captured[n].as_str())
            );
            assert_eq!(
                page.get("source").and_then(Value::text),
                Some("original recorded session receipt; no source reread")
            );
        }
        assert!(
            execute_with_identity(
                &saved.history,
                0,
                0,
                1,
                0,
                Some("read-0"),
                &budget(&cancelled),
            )
            .failed
        );
    }
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn index_paginates_ordered_reads_without_exposing_a_pending_slice() {
    let fixture = crate::state::tests::Fixture::new();
    let store = fixture.store().unwrap();
    let mut history =
        crate::session::persistence::create_in(&store, Model::Luna, None, None).unwrap();
    let id = history.record.as_ref().unwrap().id().to_owned();
    history.begin("Use recorded reads".into()).unwrap();
    let calls = (0..80)
        .map(|n| {
            tool_tests::call(
                &format!("read-{n}"),
                "read_file",
                &format!(r#"{{"path":"record-{n}.txt"}}"#),
            )
        })
        .collect();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(calls)),
        results: (0..80)
            .map(|n| Receipt {
                call_id: format!("read-{n}"),
                output: format!("original-{n}"),
                summary: "read_file / captured".into(),
                image: None,
            })
            .collect(),
        accepted: true,
        ..Default::default()
    });
    history.checkpoint().unwrap();
    history.projection.step = 1;
    history.projection.summary = "summarized".into();
    history.checkpoint().unwrap();
    history.release_projected();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "pending-read",
            "read_file",
            r#"{"path":"pending.txt"}"#,
        )])),
        results: vec![Receipt {
            call_id: "pending-read".into(),
            output: "pending evidence".into(),
            summary: "read_file / pending".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    history.projection.pending = Some(Pending {
        record: 2,
        offset: 1,
        summary: "partial slice".into(),
    });
    history.checkpoint().unwrap();
    drop(history);
    let saved = crate::session::persistence::load(&store, &id, true).unwrap();
    assert_eq!(saved.history.projection.pending.as_ref().unwrap().offset, 1);
    let cancelled = AtomicBool::new(false);
    let mut cursor = (0, 0, 0);
    let mut ids = Vec::new();
    let mut pages = 0;
    loop {
        let output = index(
            &saved.history,
            cursor.0,
            cursor.1,
            cursor.2,
            &budget(&cancelled),
        );
        assert!(!output.failed, "{}", output.text);
        assert!(output.text.len() <= 8 * 1024);
        let page = json::parse(&output.text, Default::default()).unwrap();
        for entry in page.get("entries").and_then(Value::array).unwrap() {
            ids.push(
                entry
                    .get("recall_address")
                    .and_then(|address| address.get("expected_call_id"))
                    .and_then(Value::text)
                    .unwrap()
                    .to_owned(),
            );
        }
        pages += 1;
        let next = page.get("next").unwrap();
        if matches!(next, Value::Null) {
            break;
        }
        cursor = (
            next.get("turn").and_then(Value::unsigned).unwrap() as usize,
            next.get("step").and_then(Value::unsigned).unwrap() as usize,
            next.get("receipt").and_then(Value::unsigned).unwrap() as usize,
        );
    }
    assert!(pages > 1);
    assert_eq!(
        ids,
        (0..80).map(|n| format!("read-{n}")).collect::<Vec<_>>()
    );
    assert_eq!(saved.history.projection.pending.as_ref().unwrap().offset, 1);
    cancelled.store(true, Ordering::Release);
    assert!(index(&saved.history, 0, 0, 0, &budget(&cancelled)).failed);
}

#[test]
fn resident_legacy_shape_and_failed_compaction_boundary_remain_bounded() {
    let mut history = History::default();
    history.begin("Inspect a saved read".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "legacy-read",
            "read_file",
            r#"{"path":"legacy.txt"}"#,
        )])),
        results: vec![Receipt {
            call_id: "legacy-read".into(),
            output: "OLD".into(),
            summary: "read_file / saved".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    history.projection.failed = true;
    let cancelled = AtomicBool::new(false);
    let failed_page = index(&history, 0, 0, 0, &budget(&cancelled));
    assert!(!failed_page.failed);
    let value = json::parse(&failed_page.text, Default::default()).unwrap();
    assert!(
        value
            .get("entries")
            .and_then(Value::array)
            .unwrap()
            .is_empty()
    );
    history.projection.failed = false;
    history.projection.step = 1;
    let covered = index(&history, 0, 0, 0, &budget(&cancelled));
    assert!(!covered.failed);
    assert!(covered.text.contains("legacy-read"));
    assert!(!covered.text.contains("OLD"));
    assert!(index(&history, 0, 2, 0, &budget(&cancelled)).failed);
}

#[test]
fn indexed_page_is_pinned_until_a_following_accepted_response() {
    let mut history = History::default();
    history.begin("Use a saved read".into()).unwrap();
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(vec![tool_tests::call(
            "source-read",
            "read_file",
            r#"{"path":"source.txt"}"#,
        )])),
        results: vec![Receipt {
            call_id: "source-read".into(),
            output: "recorded value".into(),
            summary: "read_file / captured".into(),
            image: None,
        }],
        accepted: true,
        ..Default::default()
    });
    history.projection.step = 1;
    let cancelled = AtomicBool::new(false);
    let page = index(&history, 0, 0, 0, &budget(&cancelled));
    assert!(!page.failed);
    let call = tool_tests::call(
        "index-call",
        "recall_receipts",
        r#"{"mode":"index","turn":0,"step":0,"receipt":0,"offset":0,"expected_call_id":"placeholder"}"#,
    );
    assert!(matches!(
        Prepared::parse(&call.name, &call.arguments),
        Ok(Prepared::Recall {
            index: true,
            offset: 0,
            expected_call_id: None,
            ..
        })
    ));
    let mut result = Receipt {
        call_id: "index-call".into(),
        output: page.text,
        summary: "recall_receipts / indexed saved reads".into(),
        image: None,
    };
    assert!(admitted(&call, &result));
    result.output = result.output.replace("\"receipt\":0", "\"receipt\":1");
    assert!(!admitted(&call, &result));
    assert!(
        Prepared::parse(
            "recall_receipts",
            &json::parse(
                r#"{"mode":"index","turn":0,"step":0,"offset":1,"expected_call_id":"placeholder"}"#,
                Default::default(),
            )
            .unwrap(),
        )
        .is_err()
    );
}

#[test]
fn index_keeps_original_indices_when_effects_and_unexecuted_reads_are_skipped() {
    let mut history = History::default();
    history.begin("Use recorded observations".into()).unwrap();
    let calls = vec![
        tool_tests::call("first", "read_file", r#"{"path":"first.txt"}"#),
        tool_tests::call(
            "effect",
            "edit_file",
            r#"{"path":"first.txt","old_text":"a","new_text":"b"}"#,
        ),
        tool_tests::call("second", "read_file", r#"{"path":"second.txt"}"#),
        tool_tests::call("unexecuted", "read_file", r#"{"path":"later.txt"}"#),
        tool_tests::call("last", "read_file", r#"{"path":"last.txt"}"#),
    ];
    history.turns[0].steps.push(Step {
        response: Some(tool_tests::calls_response(calls)),
        results: [
            ("first", "ORIGINAL-FIRST", "read_file / saved"),
            ("effect", "applied", "edit_file / applied"),
            ("second", "ORIGINAL-SECOND", "read_file / saved"),
            ("unexecuted", "", "Not executed"),
            ("last", "ORIGINAL-LAST", "read_file / saved"),
        ]
        .into_iter()
        .map(|(call_id, output, summary)| Receipt {
            call_id: call_id.into(),
            output: output.into(),
            summary: summary.into(),
            image: None,
        })
        .collect(),
        accepted: true,
        ..Default::default()
    });
    history.projection.step = 1;
    let cancelled = AtomicBool::new(false);
    let output = index(&history, 0, 0, 0, &budget(&cancelled));
    assert!(!output.failed);
    let value = json::parse(&output.text, Default::default()).unwrap();
    let addresses = value
        .get("entries")
        .and_then(Value::array)
        .unwrap()
        .iter()
        .map(|entry| entry.get("recall_address").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        addresses
            .iter()
            .map(|address| address.get("receipt").and_then(Value::unsigned).unwrap())
            .collect::<Vec<_>>(),
        vec![0, 2, 4]
    );
    assert_eq!(
        addresses
            .iter()
            .map(|address| address
                .get("expected_call_id")
                .and_then(Value::text)
                .unwrap())
            .collect::<Vec<_>>(),
        vec!["first", "second", "last"]
    );
    assert!(matches!(value.get("next"), Some(Value::Null)));
}
