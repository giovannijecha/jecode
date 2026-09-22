use super::{
    Key,
    block::BlockLayout,
    markdown,
    model::{Block, Model},
    render::Renderer,
    resize::Resize,
    schedule::PaintSchedule,
    view::Layout,
};
use std::time::{Duration, Instant};

#[test]
fn fragmented_markdown_matches_whole_render_including_styles_and_controls() {
    for source in [
        "# Heading\n**bold** and `code`\n```rust\nlet a = \"hi\";\n```\nAfter.",
        "```\n```\n```rust\n// comment\r\nlet x = 1;\n```\n\n",
        "café 👩‍💻 中文 **e\u{301}**\n\x1b[2J\u{202e}\u{009b}\ttext",
        "**unterminated\n`inline\n```rust\nlet incomplete = \"",
        "\n\r\n\r",
    ] {
        for width in [1, 2, 24, 80] {
            let mut layout = BlockLayout::new(width);
            let mut block = Block {
                speaker: "Demo",
                text: String::new(),
            };
            for end in source.char_indices().map(|(n, _)| n).chain([source.len()]) {
                block.text = source[..end].into();
                assert_eq!(
                    layout.rows(&block),
                    markdown::render(&block.text, width),
                    "width {width}, prefix {:?}",
                    block.text
                );
                // Unchanged frames must preserve the same text and styling too.
                assert_eq!(layout.rows(&block), markdown::render(&block.text, width));
            }
        }
    }
}

#[test]
fn edits_truncation_and_speaker_changes_invalidate_cached_state() {
    let mut model = Model::new(Instant::now());
    let mut layout = Layout::default();
    for (speaker, source) in [
        ("Demo", "```rust\nlet x = 1;\n"),
        ("Demo", "```rust\nlet y = 2;\n"),
        ("Demo", "```rust\n"),
        ("Demo", ""),
        ("You", ""),
        ("You", "**literal**"),
        ("Demo", "**literal**"),
        ("Demo", "ordinary\ntext"),
    ] {
        model.blocks[0] = Block {
            speaker,
            text: source.into(),
        };
        assert_eq!(
            layout.frame(&model, 80, 30),
            Layout::default().frame(&model, 80, 30)
        );
    }
    model.blocks.clear();
    layout.frame(&model, 80, 30);
    model.blocks.push(Block {
        speaker: "You",
        text: "new conversation".into(),
    });
    assert_eq!(
        layout.frame(&model, 80, 30),
        Layout::default().frame(&model, 80, 30)
    );
}

#[test]
fn completed_lines_are_not_parsed_again_when_tail_or_composer_changes() {
    let mut layout = BlockLayout::new(80);
    let mut block = Block {
        speaker: "Demo",
        text: "completed **line**\n".repeat(200),
    };
    layout.rows(&block);
    let initial = layout.parsed_bytes;
    for _ in 0..100 {
        layout.rows(&block);
    }
    assert_eq!(layout.parsed_bytes, initial);
    block.text.push_str("tail");
    layout.rows(&block);
    assert_eq!(layout.parsed_bytes - initial, 4);
    block.text.push('\n');
    layout.rows(&block);
    assert_eq!(layout.parsed_bytes - initial, 8);
    let committed = layout.parsed_bytes;
    block.text.push_str("new");
    layout.rows(&block);
    assert_eq!(layout.parsed_bytes - committed, 3);
}

#[test]
fn scheduled_paint_keeps_final_failure_or_cancellation_and_draft_after_resize() {
    for cancel in [false, true] {
        let now = Instant::now();
        let mut model = Model::new(now);
        model.input(Key::Text("/error".into()), now);
        model.input(Key::Enter, now);
        let mut paint = PaintSchedule::default();
        let mut resize = Resize::default();
        let mut layout = Layout::default();
        let mut renderer = Renderer::default();
        let mut final_frame = Vec::new();
        let mut frames = 0;
        let mut displayed = (0, 0);
        paint.request();
        for n in 0..1000 {
            let time = now + Duration::from_millis(n);
            let width = if n < 50 { 80 } else { 60 };
            if n == 50 {
                paint.request();
            }
            if n == 130 {
                model.input(Key::Text("draft-kept".into()), time);
                paint.request();
            }
            if n == 150 && cancel {
                model.input(Key::Escape, time);
                paint.request();
            }
            if model.tick(time) {
                paint.request();
            }
            let region = resize.region((width, 30), displayed, time);
            if paint.ready(time)
                && let Some(region) = region
            {
                final_frame = match region {
                    super::resize::Region::Transcript => layout.frame(&model, width, 30),
                    super::resize::Region::Composer => {
                        renderer.with_chrome(super::view::chrome(&model, width, 30))
                    }
                };
                renderer.draw(final_frame.clone(), (width, 30), false);
                paint.painted(time);
                displayed = (width, 30);
                if region == super::resize::Region::Composer {
                    paint.request();
                }
                frames += 1;
            }
        }
        assert!(!model.streaming());
        assert!(!paint.ready(now + Duration::from_secs(2)));
        assert!(!model.blocks.last().unwrap().text.is_empty());
        assert_eq!(model.editor.text, "draft-kept");
        let shown: String = final_frame.iter().map(|r| r.paint(false)).collect();
        assert_eq!(shown.matches("draft-kept").count(), 1);
        assert!(shown.contains(if cancel {
            "Interrupted /"
        } else {
            "Stream failed /"
        }));
        assert!(frames < 40, "idle polling must not produce frames");
    }
}
