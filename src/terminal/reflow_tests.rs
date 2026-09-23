use super::vt;
use super::{
    Key,
    model::{Block, Model},
    render::Renderer,
    view::Layout,
};

#[test]
fn tool_rounds_keep_even_spacing_and_one_transcript_through_resize() {
    let mut model = Model::new(std::time::Instant::now());
    model.blocks = vec![Block {
        speaker: "You",
        text: "Spiegami come è organizzato il progetto.".into(),
    }];
    model.editor.insert("draft-kept");
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    let mut terminal = vt::Screen::new(120, 24);
    terminal.feed(&renderer.draw(layout.frame(&model, 120, 24), (120, 24), false));
    for (n, width) in [60, 40, 100, 55, 80].into_iter().enumerate() {
        terminal.resize(width, 24);
        model.blocks.push(Block {
            speaker: "Assistant",
            text: String::new(),
        });
        terminal.feed(&renderer.draw(layout.frame(&model, width, 24), (width, 24), false));
        model.blocks.push(Block {
            speaker: "Tool",
            text: format!("read_file / src/file-{n}.rs"),
        });
        terminal.feed(&renderer.draw(layout.frame(&model, width, 24), (width, 24), false));
        model
            .blocks
            .last_mut()
            .unwrap()
            .text
            .push_str("\n  12 lines");
        terminal.feed(&renderer.draw(layout.frame(&model, width, 24), (width, 24), false));
    }
    model.blocks.push(Block {
        speaker: "Assistant",
        text: "# Structure\nA **small** Rust project.".into(),
    });
    terminal.feed(&renderer.draw(layout.frame(&model, 80, 24), (80, 24), false));
    let shown = terminal.text();
    let compact: String = shown.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(
        compact
            .matches("Spiegamicomeèorganizzatoilprogetto.")
            .count(),
        1
    );
    for n in 0..5 {
        assert_eq!(
            shown
                .matches(&format!("read_file / src/file-{n}.rs"))
                .count(),
            1
        );
    }
    assert_eq!(shown.matches("  12 lines").count(), 5);
    assert_eq!(shown.matches("draft-kept").count(), 1);
    assert_eq!(shown.matches("A small Rust project.").count(), 1);
    assert!(!shown.contains("12 lines\n\n\n"), "extra tool gap: {shown}");
}

#[test]
fn drag_updates_only_composer_then_flushes_source_once_after_settling() {
    use super::{
        resize::{Region, Resize},
        view,
    };
    use std::time::{Duration, Instant};
    let now = Instant::now();
    let mut model = Model::new(now);
    model.blocks.push(Block {
        speaker: "Demo",
        text: "retained paragraph\n".repeat(40),
    });
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    let mut terminal = vt::Screen::new(120, 36);
    terminal.feed(&renderer.draw(layout.frame(&model, 120, 36), (120, 36), false));
    let mut resize = Resize::default();
    let mut displayed = (120, 36);
    assert_eq!(
        resize.region(displayed, displayed, now),
        Some(Region::Transcript)
    );
    model
        .blocks
        .last_mut()
        .unwrap()
        .text
        .push_str("pending source");
    model.editor.insert("draft-kept");
    for (n, width) in [80, 62, 95, 70, 50].into_iter().enumerate() {
        let time = now + Duration::from_millis((n as u64 + 1) * 40);
        let size = (width, 36);
        terminal.resize(size.0, size.1);
        assert_eq!(resize.region(size, displayed, time), Some(Region::Composer));
        let frame = renderer.with_chrome(view::chrome(&model, size.0, size.1));
        let output = renderer.draw(frame, size, false);
        assert!(!output.contains("retained paragraph"));
        assert!(!output.contains("pending source"));
        terminal.feed(&output);
        let shown = terminal.text();
        assert_eq!(
            shown.lines().filter(|line| line.contains('─')).count(),
            2,
            "{shown}"
        );
        assert_eq!(shown.matches("draft-kept").count(), 1, "{shown}");
        assert_eq!(shown.matches("retained paragraph").count(), 40, "{shown}");
        displayed = size;
    }
    assert_eq!(
        resize.region(displayed, displayed, now + Duration::from_millis(250)),
        None
    );
    assert_eq!(
        resize.region(displayed, displayed, now + Duration::from_millis(300)),
        Some(Region::Transcript)
    );
    terminal.feed(&renderer.draw(layout.frame(&model, 50, 36), (50, 36), false));
    let shown = terminal.text();
    assert_eq!(shown.matches("retained paragraph").count(), 40, "{shown}");
    assert_eq!(shown.matches("pending source").count(), 1, "{shown}");
    assert_eq!(shown.matches("draft-kept").count(), 1, "{shown}");
}

#[test]
fn account_activity_updates_through_chrome_only_resizes_without_replaying_history() {
    use super::{account, view};
    use crate::session::{self, End, Event, Metrics};
    let mut model = account::model(session::Model::Luna, None);
    account::event(&mut model, Event::Ready);
    let mut session = session::tests::ready_fixture();
    account::input(
        &mut model,
        Key::Text("Inspect fixture".into()),
        &mut session,
    );
    account::input(&mut model, Key::Enter, &mut session);
    model.editor.insert("draft-kept");
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    let mut terminal = vt::Screen::new(100, 30);
    terminal.feed(&renderer.draw(layout.frame(&model, 100, 30), (100, 30), false));
    let events = [
        (Event::RequestStarted, "Waiting for model", (80, 30)),
        (Event::Thinking, "Thinking", (55, 24)),
        (
            Event::ToolStarted {
                name: "read_file",
                path: "src/main.rs".into(),
            },
            "Exploring workspace",
            (100, 30),
        ),
        (
            Event::ToolFinished {
                summary: "12 lines".into(),
                failed: false,
                limited: false,
            },
            "Exploring workspace",
            (60, 24),
        ),
        (Event::RequestStarted, "Exploring workspace", (90, 30)),
        (Event::Thinking, "Exploring workspace", (55, 24)),
    ];
    for (event, activity, size) in events {
        account::event(&mut model, event);
        terminal.resize(size.0, size.1);
        let frame = renderer.with_chrome(view::chrome(&model, size.0, size.1));
        let upper = frame.iter().position(|r| r.text.starts_with('─')).unwrap();
        assert!(frame[..upper].iter().any(|r| r.text.contains(activity)));
        assert!(
            frame[upper + 1..]
                .iter()
                .all(|r| !r.text.contains(activity))
        );
        let output = renderer.draw(frame, size, false);
        assert!(
            !output.contains("Inspect fixture"),
            "transcript replayed: {output}"
        );
        terminal.feed(&output);
        let shown = terminal.text();
        assert_eq!(shown.matches("Inspect fixture").count(), 1, "{shown}");
        assert_eq!(shown.matches(activity).count(), 1, "{shown}");
        assert_eq!(shown.matches("draft-kept").count(), 1, "{shown}");
    }
    account::event(
        &mut model,
        Event::Text("The file has one entry point.".into()),
    );
    account::event(
        &mut model,
        Event::Finished(End::Complete, Metrics::default()),
    );
    for size in [(100, 30), (55, 24), (90, 30)] {
        terminal.resize(size.0, size.1);
        let frame = layout.frame(&model, size.0, size.1);
        terminal.feed(&renderer.draw(frame, size, false));
        let shown = terminal.text();
        for text in ["Inspect fixture", "Explored workspace", "draft-kept"] {
            assert_eq!(shown.matches(text).count(), 1, "{shown}");
        }
        assert!(!shown.contains("Exploring workspace"), "{shown}");
        assert!(!shown.contains("Thinking"), "{shown}");
        assert_eq!(shown.lines().filter(|line| line.contains('─')).count(), 2);
    }
    assert!(
        model
            .blocks
            .iter()
            .any(|block| block.text.contains("12 lines"))
    );
}

#[test]
fn streaming_continues_through_reflow_without_replaying_completed_text() {
    use std::time::{Duration, Instant};
    let now = Instant::now();
    let mut model = Model::new(now);
    model.input(Key::Text("/long".into()), now);
    model.input(Key::Enter, now);
    model.editor.insert("draft-kept");
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    let mut terminal = vt::Screen::new(120, 30);
    let mut size = (120, 30);
    for n in 1..=850 {
        let changed_size = match n {
            120 => Some((46, 38)),
            300 => Some((100, 24)),
            500 => Some((46, 38)),
            _ => None,
        };
        if let Some(next) = changed_size {
            terminal.resize(next.0, next.1);
            size = next;
        }
        model.tick(now + Duration::from_millis(n * 40));
        terminal.feed(&renderer.draw(layout.frame(&model, size.0, size.1), size, false));
    }
    assert!(!model.streaming());
    let shown = terminal.text();
    let compact: String = shown.chars().filter(|ch| !ch.is_whitespace()).collect();
    for paragraph in model
        .blocks
        .last()
        .unwrap()
        .text
        .split("\n\n")
        .filter(|s| !s.is_empty())
    {
        let expected: String = paragraph.chars().filter(|ch| !ch.is_whitespace()).collect();
        assert_eq!(compact.matches(&expected).count(), 1, "{shown}");
    }
    assert_eq!(compact.matches("draft-kept").count(), 1);
}

#[test]
fn resize_preserves_every_emitted_paragraph_and_the_draft() {
    let mut model = Model::new(std::time::Instant::now());
    let paragraphs: Vec<String> = (1..=28).map(|n| format!("{n:02}. A useful harness keeps the task visible, streams progress and makes every effect explicit. Resize the window or use the terminal scrollback while this text arrives.")).collect();
    model.blocks.push(Block {
        speaker: "Demo",
        text: paragraphs.join("\n\n"),
    });
    model.editor.insert("draft-kept");
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    let mut terminal = vt::Screen::new(120, 30);
    terminal.feed(&renderer.draw(layout.frame(&model, 120, 30), (120, 30), false));
    for (width, height) in [
        (119, 30),
        (90, 32),
        (60, 35),
        (46, 38),
        (120, 18),
        (46, 38),
        (120, 30),
    ] {
        terminal.resize(width, height);
        let output = renderer.draw(layout.frame(&model, width, height), (width, height), false);
        assert!(
            !output.contains("useful harness"),
            "resize replayed conversation"
        );
        assert!(!output.contains("\x1b[2J"));
        assert!(!output.contains("\x1b[3J"));
        terminal.feed(&output);
        let shown = terminal.text();
        let compact: String = shown.chars().filter(|ch| !ch.is_whitespace()).collect();
        let mut last = 0;
        for paragraph in &paragraphs {
            let expected: String = paragraph.chars().filter(|ch| !ch.is_whitespace()).collect();
            assert_eq!(
                compact.matches(&expected).count(),
                1,
                "{width}x{height}: {shown}"
            );
            let position = compact.find(&expected).unwrap();
            assert!(position >= last);
            last = position;
        }
        assert_eq!(compact.matches("draft-kept").count(), 1, "{shown}");
    }
    model
        .blocks
        .last_mut()
        .unwrap()
        .text
        .push_str("\n\nNew output after resize.");
    let output = renderer.draw(layout.frame(&model, 120, 30), (120, 30), false);
    assert!(output.contains("New output after resize."));
    assert!(!output.contains("useful harness"));
    terminal.feed(&output);
    assert_eq!(
        terminal.text().matches("New output after resize.").count(),
        1
    );
}
