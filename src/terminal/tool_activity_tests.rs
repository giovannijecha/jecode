use super::vt;
use super::{
    Key,
    model::{Block, Model},
    render::Renderer,
    tool_activity::Outcome,
    view::{self, Layout},
};
use std::time::{Duration, Instant};

#[test]
fn many_reads_become_one_stable_summary_without_losing_receipts() {
    let now = Instant::now();
    let mut model = Model::new(now);
    for n in 0..12 {
        let index = model.start_tool("read_file", format!("src/file-{n}.rs"), now);
        model.finish_tool(index, "10 lines", false, false, now);
        model.blocks.push(Block {
            speaker: "Assistant",
            text: String::new(),
        });
    }
    assert_eq!(model.tools.groups.len(), 1);
    assert_eq!(model.tools.active().unwrap().counts(), "12 reads");
    let mut layout = Layout::default();
    let active = layout.frame(&model, 80, 24);
    assert!(
        !active
            .iter()
            .any(|r| !r.transient && r.text.contains("read_file"))
    );
    model
        .tools
        .close(&model.blocks, false, now + Duration::from_secs(2));
    let frame = layout.frame(&model, 80, 24);
    assert_eq!(
        frame
            .iter()
            .filter(|r| r.text.contains("Explored workspace"))
            .count(),
        1
    );
    assert!(frame.iter().any(|r| r.text.contains("12 reads · 2.0s")));
    assert_eq!(
        model
            .blocks
            .iter()
            .filter(|b| b.speaker == "Tool" && b.text.contains("10 lines"))
            .count(),
        12
    );
    assert_eq!(model.tools.groups[0].calls.len(), 12);
    assert!(!model.tools.tick(now + Duration::from_secs(100)));
    assert_eq!(frame, layout.frame(&model, 80, 24));
}

#[test]
fn limited_failed_and_interrupted_results_remain_visible_and_are_not_successes() {
    let now = Instant::now();
    let mut model = Model::new(now);
    let first = model.start_tool("read_file", "large.rs".into(), now);
    model.finish_tool(first, "80 lines / more available", false, true, now);
    let second = model.start_tool("search_text", "private".into(), now);
    model.finish_tool(second, "permission denied", true, false, now);
    model.start_tool("list_files", "slow".into(), now);
    model
        .tools
        .close(&model.blocks, true, now + Duration::from_secs(1));
    let rows = view::frame(&model, 120, 40);
    let text = rows
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for expected in [
        "Exploration interrupted",
        "1 failed · 1 limited · 1 read",
        "large.rs",
        "more available",
        "private",
        "permission denied",
        "slow",
        "no result received",
    ] {
        assert!(text.contains(expected), "{expected}: {text}");
    }
    assert!(!text.contains("1 listing"));
    assert!(!text.contains("1 search"));
    assert_eq!(model.tools.groups[0].calls[2].outcome, Outcome::Interrupted);
}

#[test]
fn unknown_tools_close_the_group_and_remain_individual() {
    let now = Instant::now();
    let mut model = Model::new(now);
    let index = model.start_tool("read_file", "first.rs".into(), now);
    model.finish_tool(index, "2 lines", false, false, now);
    let other = model.start_tool("rejected tool", String::new(), now);
    model.finish_tool(other, "unsupported tool", true, false, now);
    let next = model.start_tool("read_file", "next.rs".into(), now);
    model.finish_tool(next, "3 lines", false, false, now);
    model.tools.close(&model.blocks, false, now);
    let rows = view::frame(&model, 80, 24);
    assert_eq!(model.tools.groups.len(), 2);
    assert_eq!(
        rows.iter()
            .filter(|r| r.text.contains("Explored workspace"))
            .count(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.text.contains("rejected tool"))
            .count(),
        1
    );
    assert!(rows.iter().any(|r| r.text.contains("unsupported tool")));
}

#[test]
fn activity_text_keeps_its_column_across_frames_and_completion() {
    let now = Instant::now();
    for reduced in [false, true] {
        for failed in [false, true] {
            let mut model = Model::new(now);
            model.tools.reduced_motion = reduced;
            let index = model.start_tool("read_file", "src/main.rs".into(), now);
            for frame in 0..20 {
                model.tick(now + Duration::from_millis(frame * 80));
                let rows = super::tool_view::active(&model, 80);
                let prefix = rows[0].text.split("Exploring").next().unwrap();
                // UI symbols occupy one native cell; count independently of width().
                assert_eq!(prefix.chars().count(), 2, "{}", rows[0].text);
                assert_eq!(super::text::width(prefix), 2);
                for row in &rows[1..] {
                    assert_eq!(row.text.len() - row.text.trim_start().len(), 2);
                }
                assert!(!rows[0].paint(false).contains('\x1b'));
                assert_eq!(model.tools.marker().chars().count(), 1);
                assert!(matches!(
                    model.tools.marker().chars().next(),
                    Some('\u{2800}'..='\u{28ff}')
                ));
            }
            model.finish_tool(index, "result", failed, false, now);
            model.tools.close(&model.blocks, false, now);
            let summary = &model.tools.groups[0].summary.as_ref().unwrap().text;
            assert_eq!(summary.split("Explor").next().unwrap().chars().count(), 2);
        }
    }
}

#[test]
fn animation_is_bounded_stops_when_idle_and_respects_reduced_motion() {
    let now = Instant::now();
    for reduced in [false, true] {
        let mut model = Model::new(now);
        model.tools.reduced_motion = reduced;
        assert!(!model.tick(now));
        model.start_tool("read_file", "file.rs".into(), now);
        let first = model.tools.marker();
        let mut paints = 0;
        for n in 1..=999 {
            paints += usize::from(model.tick(now + Duration::from_millis(n)));
        }
        if reduced {
            assert_eq!(paints, 0);
            assert_eq!(model.tools.marker(), first);
        } else {
            assert_eq!(paints, 12);
            assert_ne!(model.tools.marker(), first);
        }
        model
            .tools
            .close(&model.blocks, true, now + Duration::from_secs(1));
        assert!(!model.tick(now + Duration::from_secs(2)));
    }
}

#[test]
fn active_status_tracks_provider_wait_and_stopping_and_stays_bounded() {
    let now = Instant::now();
    let mut model = Model::new(now);
    let index = model.start_tool(
        "read_file",
        "café/\x1b[2J/unusually-long-path.rs".repeat(20),
        now,
    );
    model.editor.insert(&"draft ".repeat(50));
    for width in [2, 10, 25, 40, 80, 140] {
        for height in [2, 9, 12, 24] {
            let rows = view::chrome(&model, width, height);
            assert!(rows.len() < height, "{width}x{height}: {} rows", rows.len());
            for row in rows {
                assert!(row.transient);
                assert!(super::text::width(&row.text) < width);
                assert!(!row.paint(false).contains('\x1b'));
                row.paint(true);
            }
        }
    }
    model.tools.waiting("Stopping / waiting for cleanup");
    assert!(
        super::tool_view::active(&model, 100)
            .iter()
            .any(|r| r.text.contains("Stopping"))
    );
    model.finish_tool(index, "2 lines", false, false, now);
    model.tools.waiting("Waiting for model");
    let rows = super::tool_view::active(&model, 100);
    assert!(rows.iter().any(|r| r.text.contains("Waiting for model")));
    assert!(!rows.iter().any(|r| r.text.contains("Reading")));
}

#[test]
fn grouped_demo_animates_through_resize_and_commits_once_on_finish_or_cancel() {
    let now = Instant::now();
    for prompt in ["/tools", "/tools-error"] {
        for cancel in [false, true] {
            let mut model = Model::new(now);
            model.input(Key::Text(prompt.into()), now);
            model.input(Key::Enter, now);
            model.editor.insert("draft-kept");
            let mut renderer = Renderer::default();
            let mut layout = Layout::default();
            let mut terminal = vt::Screen::new(120, 30);
            let mut size = (120, 30);
            for n in 0..=360 {
                let at = now + Duration::from_millis(n * 20);
                if matches!(n, 20 | 90 | 180 | 300) {
                    size = match n {
                        20 => (45, 30),
                        90 => (120, 18),
                        180 => (55, 30),
                        _ => (120, 30),
                    };
                    terminal.resize(size.0, size.1);
                }
                model.tick(at);
                if cancel && n == 110 {
                    model.input(Key::Escape, at);
                }
                terminal.feed(&renderer.draw(layout.frame(&model, size.0, size.1), size, false));
            }
            assert!(!model.streaming());
            let shown = terminal.text();
            assert!(
                !shown.contains("Exploring workspace"),
                "active group leaked into scrollback: {shown}"
            );
            assert_eq!(shown.matches("draft-kept").count(), 1, "{shown}");
            let expected = if cancel {
                "Exploration interrupted"
            } else if prompt.ends_with("error") {
                "Exploration finished with errors"
            } else {
                "Explored workspace"
            };
            assert_eq!(shown.matches(expected).count(), 1, "{shown}");
            if !cancel && prompt.ends_with("error") {
                assert!(shown.contains("permission denied"));
            }
            assert!(
                renderer
                    .draw(layout.frame(&model, size.0, size.1), size, false)
                    .is_empty()
            );
        }
    }
}
