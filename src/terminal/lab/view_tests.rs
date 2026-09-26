use super::*;
use crate::terminal::lab::activity_view::Phase;
use crate::terminal::lab::caps::ColorDepth;

const CAPS: Caps = Caps {
    color: ColorDepth::TrueColor,
    ascii: false,
    reduced_motion: true,
};

fn texts(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|row| row.text.as_str()).collect()
}

#[test]
fn idle_frame_keeps_a_single_blank_above_the_composer() {
    let footer = Footer::default();
    let screen = Screen {
        blocks: &[],
        live: None,
        activity: None,
        notice: None,
        draft: "",
        cursor: 0,
        suggestion: 0,
        footer: &footer,
        picker: None,
        queued: &[],
        expanded: false,
    };
    let rows = frame(&screen, 20, &CAPS, 0);
    assert_eq!(rows.len(), 5);
    assert_eq!(texts(&rows[..2]), ["", "────────────────────"]);
    assert!(rows.iter().all(|row| row.transient));
}

#[test]
fn streaming_frame_shows_caret_and_activity() {
    let footer = Footer::default();
    let blocks = [Block::User("go".into())];
    let screen = Screen {
        blocks: &blocks,
        live: Some("Working on"),
        activity: Some(Activity {
            phase: Phase::Streaming,
            elapsed_ms: 1_000,
            tokens: 2,
        }),
        draft: "",
        cursor: 0,
        suggestion: 0,
        notice: None,
        footer: &footer,
        picker: None,
        queued: &[],
        expanded: false,
    };
    let rows = frame(&screen, 40, &CAPS, 0);
    let live = rows.iter().position(|row| row.text == " Working on▍");
    assert_eq!(live, Some(4), "{:?}", texts(&rows));
    assert!(!rows[4].transient && rows[5].transient);
    assert!(rows[6].text.starts_with(" ⣿ Streaming · 1s · 2 tok"));
}

#[test]
fn queued_messages_wait_above_the_composer_in_one_row_each() {
    let footer = Footer::default();
    let queued = ["next please".to_string(), "a\nb".to_string()];
    let screen = Screen {
        blocks: &[],
        live: None,
        activity: None,
        notice: None,
        draft: "",
        cursor: 0,
        suggestion: 0,
        footer: &footer,
        picker: None,
        expanded: false,
        queued: &queued,
    };
    let rows = frame(&screen, 22, &CAPS, 0);
    assert_eq!(
        texts(&rows[..3]),
        ["", " › next ple… · queued", " › a … · queued"]
    );
    assert!(rows[1].spans.iter().all(|span| span.1 != Tone::Accent));
}

#[test]
fn an_eight_row_window_keeps_queue_count_and_editor_visible() {
    let footer = Footer {
        cwd: "C:/a/very/long/workspace/path".into(),
        model: "gpt-5.6-terra".into(),
        effort: "provider default".into(),
    };
    let queued: Vec<_> = (0..8)
        .map(|index| format!("queued message {index}"))
        .collect();
    let screen = Screen {
        blocks: &[],
        live: None,
        activity: Some(Activity {
            phase: Phase::Waiting,
            elapsed_ms: 0,
            tokens: 0,
        }),
        notice: None,
        draft: "one\ntwo\nthree\nfour\nfive\nsix\nseven",
        cursor: 30,
        suggestion: usize::MAX,
        footer: &footer,
        picker: None,
        queued: &queued,
        expanded: false,
    };
    let mut layout = Layout::default();
    let rows = layout.frame_bounded(&screen, 20, 8, &CAPS, 0);
    let chrome: Vec<_> = rows.iter().filter(|row| row.transient).collect();
    assert!(chrome.len() <= 7, "{chrome:?}");
    assert!(
        chrome.iter().all(|row| text::width(&row.text) <= 20),
        "{chrome:?}"
    );
    assert!(
        chrome.iter().any(|row| row.text.contains("8 queued")),
        "{chrome:?}"
    );
    assert!(
        chrome
            .iter()
            .any(|row| row.spans.iter().any(|span| span.1 == Tone::Cursor)),
        "{chrome:?}"
    );
}

#[test]
fn a_cached_layout_matches_one_from_scratch() {
    use crate::terminal::lab::model::{Detail, Tool};
    let footer = Footer::default();
    let tool = |status| Tool {
        verb: "Run".into(),
        subject: "make".into(),
        summary: String::new(),
        status,
        elapsed_ms: 0,
        detail: Detail::Output((1..9).map(|n| format!("{n}\n")).collect()),
    };
    let steps: [(Vec<Block>, usize, bool, u64); 5] = [
        (vec![Block::User("go".into())], 30, false, 0),
        (
            vec![
                Block::User("go".into()),
                Block::Tools(vec![tool(Status::Running)]),
            ],
            30,
            false,
            0,
        ),
        (
            vec![
                Block::User("go".into()),
                Block::Tools(vec![tool(Status::Running)]),
            ],
            30,
            false,
            500,
        ),
        (
            vec![
                Block::User("go".into()),
                Block::Tools(vec![tool(Status::Done)]),
            ],
            24,
            true,
            900,
        ),
        (vec![Block::Assistant("done".into())], 24, true, 900),
    ];
    let mut layout = Layout::default();
    for (blocks, width, expanded, now) in steps {
        let screen = Screen {
            blocks: &blocks,
            live: None,
            activity: None,
            notice: None,
            draft: "",
            cursor: 0,
            suggestion: 0,
            footer: &footer,
            picker: None,
            expanded,
            queued: &[],
        };
        assert_eq!(
            layout.frame(&screen, width, &CAPS, now),
            frame(&screen, width, &CAPS, now)
        );
    }
}

#[test]
fn short_picker_keeps_the_selected_choice_and_authentication_code() {
    let footer = Footer::default();
    let picker = Picker {
        title: "Reasoning effort".into(),
        choices: (0..8)
            .map(|index| picker::Choice {
                label: format!("effort {index}"),
                detail: String::new(),
            })
            .collect(),
        query: String::new(),
        selected: 7,
    };
    let notice = Notice {
        status: Status::Warned,
        text: "Code: ABCD-EFGH\nSign in at https://example.test/activate\nWaiting for approval"
            .into(),
    };
    let screen = Screen {
        blocks: &[],
        live: None,
        activity: None,
        notice: Some(&notice),
        draft: "",
        cursor: 0,
        suggestion: usize::MAX,
        footer: &footer,
        picker: Some(&picker),
        expanded: false,
        queued: &[],
    };
    let rows = Layout::default().frame_bounded(&screen, 20, 8, &CAPS, 0);
    let chrome: Vec<_> = rows.iter().filter(|row| row.transient).collect();
    assert!(chrome.len() <= 7, "{chrome:?}");
    assert!(chrome.iter().all(|row| text::width(&row.text) <= 20));
    assert!(
        chrome.iter().any(|row| row.text.contains("ABCD-EFGH")),
        "{chrome:?}"
    );
    assert!(
        chrome
            .iter()
            .any(|row| row.tone == Tone::User && row.text.contains('8')),
        "{chrome:?}"
    );
}

#[test]
fn resize_preview_has_the_same_visible_tail_as_full_layout() {
    let footer = Footer::default();
    let mut blocks = Vec::new();
    for index in 0..200 {
        blocks.push(Block::User(format!("prompt {index}")));
        blocks.push(Block::Assistant(format!("response {index}")));
    }
    blocks.push(Block::Assistant("last line\n".repeat(120)));
    let screen = Screen {
        blocks: &blocks,
        live: None,
        activity: None,
        notice: None,
        draft: "typing",
        cursor: 6,
        suggestion: usize::MAX,
        footer: &footer,
        picker: None,
        expanded: false,
        queued: &[],
    };
    for (width, height) in [(20, 8), (40, 12), (80, 24), (120, 40)] {
        let full = Layout::default().frame_bounded(&screen, width, height, &CAPS, 0);
        let short = preview(&screen, width, height, &CAPS, 0);
        let visible = height - 1;
        assert_eq!(
            &short[short.len().saturating_sub(visible)..],
            &full[full.len().saturating_sub(visible)..],
            "{width}x{height}"
        );
    }
}
