//! Exact ANSI frames captured from the frozen owned lab at virtual time 0.
//! The fixture contains static turns, so no mock provider is needed here.
use super::{
    caps::{Caps, ColorDepth},
    composer::Footer,
    model::Block,
    render::Renderer,
    view::{Layout, Screen},
};

const SOURCE: &str = include_str!("../../../tests/fixtures/tui/transcript.jef");

fn transcript() -> (Vec<Block>, Footer) {
    let mut blocks = Vec::new();
    let mut footer = Footer {
        cwd: "~".into(),
        model: "mock".into(),
        effort: "medium".into(),
    };
    let mut open: Option<(&str, String)> = None;
    let close = |open: Option<(&str, String)>, blocks: &mut Vec<Block>| {
        if let Some((kind, body)) = open {
            let body = body.trim_matches('\n').to_owned();
            blocks.push(match kind {
                "user" => Block::User(body),
                "assistant" => Block::Assistant(body),
                _ => unreachable!(),
            });
        }
    };
    for line in SOURCE.lines() {
        if line.starts_with("@--") {
            continue;
        }
        if let Some(meta) = line.strip_prefix("@meta ") {
            let (name, value) = meta.split_once(' ').unwrap();
            match name {
                "cwd" => footer.cwd = value.into(),
                "model" => footer.model = value.into(),
                "effort" => footer.effort = value.into(),
                _ => unreachable!(),
            }
            continue;
        }
        if let Some(kind) = line.strip_prefix('@') {
            close(open.take(), &mut blocks);
            assert!(matches!(kind, "user" | "assistant"));
            open = Some((kind, String::new()));
            continue;
        }
        if let Some((_, body)) = &mut open {
            body.push_str(line);
            body.push('\n');
        } else {
            assert!(line.is_empty());
        }
    }
    close(open, &mut blocks);
    (blocks, footer)
}

#[test]
fn frozen_lab_transcript_frames_match_across_geometry_and_capabilities() {
    let (blocks, footer) = transcript();
    let cases: &[(usize, ColorDepth, bool, bool, &[u8])] = &[
        (
            20,
            ColorDepth::TrueColor,
            false,
            false,
            include_bytes!(
                "../../../tests/fixtures/tui/transcript-20-truecolor-unicode-normal.ansi"
            ),
        ),
        (
            40,
            ColorDepth::Ansi256,
            false,
            false,
            include_bytes!("../../../tests/fixtures/tui/transcript-40-256-unicode-normal.ansi"),
        ),
        (
            80,
            ColorDepth::Ansi16,
            false,
            false,
            include_bytes!("../../../tests/fixtures/tui/transcript-80-16-unicode-normal.ansi"),
        ),
        (
            120,
            ColorDepth::None,
            true,
            true,
            include_bytes!("../../../tests/fixtures/tui/transcript-120-none-ascii-reduced.ansi"),
        ),
        (
            80,
            ColorDepth::TrueColor,
            false,
            true,
            include_bytes!(
                "../../../tests/fixtures/tui/transcript-80-truecolor-unicode-reduced.ansi"
            ),
        ),
    ];
    for &(width, color, ascii, reduced_motion, expected) in cases {
        let caps = Caps {
            color,
            ascii,
            reduced_motion,
        };
        let frame = Layout::default().frame(
            &Screen {
                blocks: &blocks,
                live: None,
                activity: None,
                notice: None,
                draft: "",
                cursor: usize::MAX,
                suggestion: 0,
                footer: &footer,
                picker: None,
                expanded: false,
                queued: &[],
            },
            width,
            &caps,
            0,
        );
        let output = Renderer::default().draw(frame, (width, 24), color);
        // Real ConPTY needs autowrap disabled around full-width rows. The
        // frozen lab predates that terminal-state fix; all styled rows remain exact.
        let output = output
            .replace("\x1b[?2026h\x1b[?7l", "\x1b[?2026h")
            .replace("\x1b[?7h\x1b[?2026l", "\x1b[?2026l");
        assert_eq!(output.as_bytes(), expected, "{width} columns, {caps:?}");
    }
}
