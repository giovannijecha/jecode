//! Presentation regressions against source offsets and canonical receipts.
use super::*;
use crate::session::{self, Event};
use std::time::Duration;

#[test]
fn tab_and_word_wrap_share_source_cursor_and_display_rows() {
    let mut model = account::model(session::Model::Luna, None);
    account::event(&mut model, Event::Ready);
    model.editor.insert("\tX");
    model.editor.draft_start();
    let caps = lab::caps::Caps {
        color: lab::caps::ColorDepth::None,
        ascii: true,
        reduced_motion: true,
    };
    let rows = lab_adapter::Snapshot::from_model(&model).frame(
        &mut lab::view::Layout::default(),
        80,
        24,
        &caps,
        0,
    );
    let row = rows
        .iter()
        .find(|row| {
            row.spans
                .iter()
                .any(|(_, tone)| *tone == lab::style::Tone::Cursor)
        })
        .unwrap();
    let span = row
        .spans
        .iter()
        .find(|(_, tone)| *tone == lab::style::Tone::Cursor)
        .unwrap();
    assert_eq!(span.0.start, 3);
    model.editor.set_columns(15); // 20 terminal columns minus composer lead and cursor reserve.
    model.editor.replace("123456789012345X");
    model.editor.draft_start();
    assert!(model.editor.vertical(true));
    assert_eq!(model.editor.cursor, 15);
    assert_eq!(model.editor.text, "123456789012345X");
    let rows = lab_adapter::Snapshot::from_model(&model).frame(
        &mut lab::view::Layout::default(),
        20,
        12,
        &caps,
        0,
    );
    assert!(rows.iter().any(|row| row.text.contains("X")));
}

#[test]
fn multiline_unicode_word_wrap_and_resize_keep_grapheme_stops_in_bounds() {
    let draft = "alpha beta gamma\n  e\u{301} 👩\u{200d}💻\tZ";
    let source = editor::boundaries(draft);
    let caps = lab::caps::Caps {
        color: lab::caps::ColorDepth::None,
        ascii: true,
        reduced_motion: true,
    };
    let mut model = account::model(session::Model::Luna, None);
    account::event(&mut model, Event::Ready);
    model.editor.insert(draft);
    for width in [20, 40, 80] {
        model.editor.set_columns(width - 5);
        for &cursor in &source {
            model.editor.cursor = cursor;
            let visual = editor_visual::Visual::new(draft, width - 5);
            let stop = visual.stop(cursor);
            assert!(stop.row < visual.rows.len());
            assert!(stop.byte <= visual.rows[stop.row].len());
            let rows = lab_adapter::Snapshot::from_model(&model).frame(
                &mut lab::view::Layout::default(),
                width,
                24,
                &caps,
                0,
            );
            assert_eq!(
                rows.iter()
                    .flat_map(|row| &row.spans)
                    .filter(|(_, tone)| *tone == lab::style::Tone::Cursor)
                    .count(),
                1
            );
            assert!(rows.iter().all(|row| lab::text::width(&row.text) <= width));
        }
    }
    model.editor.cursor = draft.find("👩").unwrap();
    assert!(model.editor.insert("X"));
    assert!(model.editor.text.contains("X👩\u{200d}💻"));
}

#[test]
fn canonical_read_controls_are_safe_in_rows_and_serialized_output() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/review-read-controls-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("sample.txt"),
        "normal\rOVERWRITTEN\tcolumn\u{202e}hidden\n",
    )
    .unwrap();
    let workspace = crate::workspace::Workspace::open(&root).unwrap();
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let args = crate::json::parse(r#"{"path":"sample.txt"}"#, Default::default()).unwrap();
    let output = crate::tools::Prepared::parse("read_file", &args)
        .unwrap()
        .execute(
            &workspace,
            &crate::workspace::Budget {
                cancelled: &cancelled,
                deadline: Instant::now() + Duration::from_secs(5),
            },
        );
    assert!(!output.failed);
    assert!(output.text.contains("\\r") && output.text.contains("\\t"));
    let mut model = account::model(session::Model::Luna, None);
    let now = Instant::now();
    let index = model.start_tool("read_file", "sample.txt\tlabel".into(), now);
    model.finish_tool_output(
        index,
        "read\rcomplete",
        output.failed,
        output.limited,
        &output.text,
        now,
    );
    let caps = lab::caps::Caps {
        color: lab::caps::ColorDepth::None,
        ascii: false,
        reduced_motion: true,
    };
    let rows = lab_adapter::Snapshot::from_model(&model).frame(
        &mut lab::view::Layout::default(),
        80,
        24,
        &caps,
        0,
    );
    assert!(
        rows.iter()
            .all(|row| !row.text.contains(['\r', '\t', '\u{202e}']))
    );
    let serialized = lab::render::Renderer::default().draw(rows, (80, 24), caps.color);
    assert!(
        !serialized.contains("normal\rOVERWRITTEN")
            && !serialized.contains('\t')
            && !serialized.contains('\u{202e}')
    );
}

#[test]
fn live_index_and_restored_command_keep_details_and_history_intact() {
    let mut model = account::model(session::Model::Luna, None);
    let now = Instant::now();
    let index = model.start_tool("index_receipts", "".into(), now);
    let raw = r#"{"ok":true,"entries":[{"recall_address":{"turn":0,"step":0,"receipt":0,"offset":0,"expected_call_id":"read-1"},"call_name":"read_file","arguments":{"path":"evidence.txt"},"arguments_omitted":false}],"next":{"turn":0,"step":1,"receipt":0}}"#;
    model.finish_tool_output(index, "indexed 1 saved reads", false, true, raw, now);
    let lab::model::Detail::Output(detail) = &model.tool_details[&index].detail else {
        panic!("missing index");
    };
    assert!(
        detail.contains("read_file / recall")
            && detail.contains("evidence.txt")
            && detail.contains("Next:")
    );
    let saved = r#"{"ok":true,"status":"exited","exit_code":0,"stdout":"line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nEND_OF_STDOUT\n","stderr":"warning\n","truncated":false,"output_bytes":64,"elapsed_ms":17,"cleanup_confirmed":true}"#;
    account::event(
        &mut model,
        Event::Restored {
            id: "synthetic-session".into(),
            turns: 1,
            items: vec![session::TranscriptItem {
                role: "Tool",
                text: "saved command".into(),
                tool: Some(session::TranscriptTool {
                    name: "run_command".into(),
                    subject: "cargo test".into(),
                    summary: "Command finished / exit 0".into(),
                    output: saved.into(),
                    failed: false,
                    limited: false,
                    outcome_unknown: false,
                }),
            }],
        },
    );
    let restored = model.tool_details.values().last().unwrap();
    assert_eq!(restored.elapsed_ms, 17);
    let lab::model::Detail::Output(detail) = &restored.detail else {
        panic!("missing command");
    };
    assert!(
        detail.contains("stdout:\nline 1")
            && detail.contains("END_OF_STDOUT")
            && detail.contains("stderr:\nwarning")
    );
    model.expanded = true;
    let caps = lab::caps::Caps {
        color: lab::caps::ColorDepth::None,
        ascii: true,
        reduced_motion: true,
    };
    let rows = lab_adapter::Snapshot::from_model(&model).frame(
        &mut lab::view::Layout::default(),
        40,
        24,
        &caps,
        0,
    );
    assert!(rows.iter().any(|row| row.text.contains("END_OF_STDOUT")));
    assert_eq!(model.tool_details.values().last().unwrap().elapsed_ms, 17);
}
