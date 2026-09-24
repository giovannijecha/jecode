use super::*;

#[test]
fn catalog_failures_keep_the_pair_and_report_a_local_reason() {
    for (kind, message) in [
        (session::CatalogFailure::Unavailable, "unavailable"),
        (session::CatalogFailure::Invalid, "malformed"),
        (session::CatalogFailure::Empty, "no usable choices"),
        (session::CatalogFailure::Cancelled, "cancelled"),
    ] {
        let mut model = model(session::Model::Terra, None);
        event(&mut model, Event::CatalogFailed(kind));
        let view = model.account.as_ref().unwrap();
        assert_eq!(view.selected, session::Model::Terra);
        assert!(view.local_notice.contains(message));
        assert!(model.blocks.is_empty());
    }
}

#[test]
fn restored_history_and_context_notices_do_not_swallow_streamed_text() {
    let mut model = model(session::Model::Luna, None);
    event(
        &mut model,
        Event::Restored {
            id: "synthetic-session".into(),
            turns: 1,
            items: vec![session::TranscriptItem {
                role: "Assistant",
                text: "Saved answer".into(),
            }],
        },
    );
    event(&mut model, Event::Ready);
    model.account.as_mut().unwrap().phase = Phase::Generating;
    event(&mut model, Event::RequestStarted);
    event(&mut model, Event::ContextReport("Context unchanged".into()));
    event(&mut model, Event::Text("Following response".into()));
    assert_eq!(model.blocks.last().unwrap().text, "Following response");
    assert!(model.blocks.iter().any(|b| b.text == "Saved answer"));
    event(
        &mut model,
        Event::Guidance {
            text: "New guidance".into(),
            new_turn: false,
        },
    );
    event(&mut model, Event::Text("Guided response".into()));
    assert_eq!(model.blocks.last().unwrap().text, "Guided response");
}
use crate::session::Failure;

#[test]
fn request_boundaries_group_reads_but_assistant_text_closes_the_group() {
    let mut model = model(session::Model::Luna, None);
    model.account.as_mut().unwrap().phase = Phase::Generating;
    for (name, failed, limited) in [("read_file", false, true), ("search_text", true, false)] {
        event(&mut model, Event::RequestStarted);
        event(
            &mut model,
            Event::ToolStarted {
                name,
                path: "fixture".into(),
            },
        );
        event(
            &mut model,
            Event::ToolFinished {
                summary: if failed { "denied" } else { "more available" }.into(),
                failed,
                limited,
            },
        );
    }
    assert_eq!(model.tools.groups.len(), 1);
    event(&mut model, Event::RequestStarted);
    assert_eq!(model.tools.active().unwrap().state, "Waiting for model");
    event(&mut model, Event::Thinking);
    assert_eq!(model.tools.active().unwrap().state, "Thinking");
    event(&mut model, Event::Text("Kept between groups.".into()));
    assert!(model.tools.active().is_none());
    event(
        &mut model,
        Event::ToolStarted {
            name: "list_files",
            path: ".".into(),
        },
    );
    event(
        &mut model,
        Event::ToolFinished {
            summary: "2 entries / 0 omitted".into(),
            failed: false,
            limited: false,
        },
    );
    event(
        &mut model,
        Event::Finished(End::Complete, Metrics::default()),
    );
    assert_eq!(model.tools.groups.len(), 2);
    assert!(model.tools.active().is_none());
    let rows = super::super::view::frame(&model, 120, 40);
    let text = rows
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let first = text.find("Exploration finished with errors").unwrap();
    let middle = text.find("Kept between groups.").unwrap();
    let last = text.find("Explored workspace").unwrap();
    assert!(first < middle && middle < last);
    assert_eq!(text.matches("more available").count(), 1);
    assert_eq!(text.matches("denied").count(), 1);
}

#[test]
fn tool_activity_and_later_text_keep_distinct_stable_blocks() {
    let mut model = model(
        session::Model::Luna,
        Some(std::path::Path::new("fixture-workspace")),
    );
    model.account.as_mut().unwrap().phase = Phase::Generating;
    model.editor.insert("next draft");
    model.blocks.push(Block {
        speaker: "Assistant",
        text: String::new(),
    });
    event(&mut model, Event::Text("Inspecting the source.".into()));
    event(
        &mut model,
        Event::ToolStarted {
            name: "read_file",
            path: "src/main.rs".into(),
        },
    );
    let mut layout = super::super::view::Layout::default();
    layout.frame(&model, 120, 40);
    event(
        &mut model,
        Event::ToolFinished {
            summary: "12 lines".into(),
            failed: false,
            limited: false,
        },
    );
    event(&mut model, Event::RequestStarted);
    event(
        &mut model,
        Event::Text("The file has one entry point.".into()),
    );
    event(
        &mut model,
        Event::Finished(
            End::Complete,
            Metrics {
                requests: 2,
                tool_calls: 1,
                ..Default::default()
            },
        ),
    );
    assert_eq!(model.blocks[0].text, "Inspecting the source.");
    assert_eq!(model.blocks[1].text, "read_file / src/main.rs\n  12 lines");
    assert_eq!(model.blocks[2].text, "The file has one entry point.");
    assert_eq!(model.editor.text, "next draft");
    assert!(
        model
            .account
            .as_ref()
            .unwrap()
            .notice
            .contains("1 tools / 2 requests")
    );
    for width in [120, 55, 150] {
        let rows = layout.frame(&model, width, 40);
        for expected in [
            "Explored workspace",
            "1 read",
            "Inspecting the source.",
            "one entry point.",
        ] {
            assert_eq!(
                rows.iter()
                    .filter(|row| row.text.contains(expected))
                    .count(),
                1
            );
        }
    }
}

#[test]
fn provider_error_is_rendered_once_in_scrollback_with_no_output() {
    use crate::providers::openai_account::{ContentKind, Error, client};
    let mut model = model(session::Model::Luna, None);
    event(&mut model, Event::Ready);
    model.blocks.push(Block {
        speaker: "You",
        text: "hello".into(),
    });
    model.blocks.push(Block {
        speaker: "Assistant",
        text: String::new(),
    });
    let failure = Failure::Account(client::Error::Protocol(Error::ContentType(
        ContentKind::Html,
    )));
    event(
        &mut model,
        Event::Finished(End::Failed(failure), Metrics::default()),
    );
    let mut layout = super::super::view::Layout::default();
    for width in [150, 55, 120] {
        let rows = layout.frame(&model, width, 40);
        let outcomes: Vec<_> = rows
            .iter()
            .filter(|row| row.text.contains("account endpoint returned"))
            .collect();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].tone, super::super::style::Tone::Error);
        assert!(!outcomes[0].transient);
        assert!(!model.streaming());
    }
}

#[test]
fn storage_failure_closes_the_session_without_another_sign_in_prompt() {
    let mut model = model(session::Model::Luna, None);
    event(&mut model, Event::Ready);
    event(
        &mut model,
        Event::Finished(End::Failed(Failure::Storage), Metrics::default()),
    );
    let view = model.account.as_ref().unwrap();
    assert!(view.phase == Phase::Closed);
    assert!(view.notice.contains("check local storage"));
    assert_eq!(
        model
            .blocks
            .iter()
            .filter(|block| block.text.contains("could not be saved"))
            .count(),
        1
    );
}

#[test]
fn login_notice_never_enters_transcript_and_partial_output_survives_failure() {
    let mut model = model(session::Model::Luna, None);
    event(&mut model, Event::LoginCode("FAKE-CODE".into()));
    assert!(model.account.as_ref().unwrap().notice.contains("FAKE-CODE"));
    assert!(!model.blocks.iter().any(|b| b.text.contains("FAKE-CODE")));
    event(&mut model, Event::Ready);
    assert!(!model.account.as_ref().unwrap().notice.contains("FAKE-CODE"));
    model.account.as_mut().unwrap().phase = Phase::Generating;
    model.blocks.push(Block {
        speaker: "Assistant",
        text: String::new(),
    });
    model.editor.insert("next draft");
    event(&mut model, Event::Text("retained partial".into()));
    event(
        &mut model,
        Event::Finished(End::Failed(Failure::Cancelled), Metrics::default()),
    );
    assert_eq!(model.blocks[0].text, "retained partial");
    assert_eq!(model.editor.text, "next draft");
    assert!(!model.streaming());
    // A finished failure belongs to scrollback once, not also to the composer.
    let mut layout = super::super::view::Layout::default();
    for width in [120, 55, 150] {
        let rows = layout.frame(&model, width, 40);
        let errors: Vec<_> = rows
            .iter()
            .filter(|row| row.text.contains(&Failure::Cancelled.to_string()))
            .collect();
        assert_eq!(errors.len(), 1, "duplicate failure at width {width}");
        assert!(!errors[0].transient);
        assert_eq!(errors[0].tone, super::super::style::Tone::Error);
    }
}

#[test]
fn cancelled_login_stays_recoverable_and_outside_canonical_history() {
    let mut model = model(session::Model::Luna, None);
    model.editor.insert("preserved draft");
    event(&mut model, Event::LoginCode("SYNTHETIC-CODE".into()));
    event(&mut model, Event::LoginFailed(Failure::Cancelled));
    let view = model.account.as_ref().unwrap();
    assert!(view.signed_out());
    assert!(!view.failed);
    assert!(view.notice.contains("/login to retry"));
    assert_eq!(model.editor.text, "preserved draft");
    assert!(
        !model
            .blocks
            .iter()
            .any(|block| block.text.contains("SYNTHETIC-CODE"))
    );
}

#[test]
fn failed_login_keeps_restored_history_and_draft_visible() {
    let mut model = model(session::Model::Luna, None);
    model.editor.insert("unsent follow-up");
    event(
        &mut model,
        Event::Restored {
            id: "synthetic-session".into(),
            items: vec![session::TranscriptItem {
                role: "Assistant",
                text: "Saved answer".into(),
            }],
            turns: 1,
        },
    );
    event(
        &mut model,
        Event::LoginFailed(Failure::Account(
            crate::providers::openai_account::client::Error::Expired,
        )),
    );
    let view = model.account.as_ref().unwrap();
    assert!(view.signed_out());
    assert!(view.notice.contains("/login to retry"));
    assert_eq!(model.editor.text, "unsent follow-up");
    assert_eq!(model.blocks[0].text, "Saved answer");
}

#[test]
fn terminal_reconciliation_and_transport_failure_keep_a_visible_correction() {
    use crate::providers::openai_account::client::{Error, RequestStage};
    use crate::tls::{IoOperation, NetworkError};
    let mut model = model(session::Model::Luna, None);
    event(&mut model, Event::Ready);
    model.account.as_mut().unwrap().phase = Phase::Generating;
    model.blocks.push(Block {
        speaker: "Assistant",
        text: String::new(),
    });
    event(&mut model, Event::Text("SameSame".into()));
    event(&mut model, Event::TextReconciled("Same\n\nSame".into()));
    assert_eq!(model.blocks[0].text, "SameSame");
    assert_eq!(model.blocks[1].speaker, "Correction");
    assert_eq!(
        model.blocks[1].text,
        "# Updated response (replaces earlier text)\n\nSame\n\nSame"
    );
    let failure = Failure::Account(Error::Transport {
        stage: RequestStage::ResponseRead,
        delivery: crate::providers::openai_account::client::Delivery::Streaming,
        accepted_wire_bytes: 42,
        error: NetworkError::io(
            IoOperation::ReadRecordBody,
            &std::io::Error::from(std::io::ErrorKind::ConnectionReset),
        ),
    });
    event(
        &mut model,
        Event::Finished(End::Failed(failure), Metrics::default()),
    );
    assert_eq!(model.blocks[0].text, "SameSame");
    assert!(model.blocks[2].text.contains("response read"));
    assert!(model.blocks[2].text.contains("partial output retained"));
}
