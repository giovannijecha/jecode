use super::*;
use crate::tui::{
    activity::Activity,
    feedback::Feedback,
    motion,
    render::Renderer,
    state::{Kind, Opening, Recent},
    text,
    theme::{ACCENT, GOOD, LIT, MUTED, SELECTED_SURFACE, STRONG},
    transcript::Layout,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn state(recent: &[(&str, &str)], columns: usize, height: usize) -> State {
    State {
        width: columns,
        height,
        effort: "high".into(),
        opening: Opening {
            recent: recent
                .iter()
                .map(|(title, age)| Recent {
                    title: (*title).into(),
                    age: (*age).into(),
                })
                .collect(),
            skills: 7,
            agents: !recent.is_empty(),
        },
        ..State::default()
    }
}

#[test]
fn the_full_splash_centres_its_rows_above_the_unchanged_dock() {
    let recent = [
        ("Fix the session expiry check in auth", "2h"),
        ("Add the sleep test", "1d"),
        ("Migrate the config loader", "3d"),
    ];
    let state = state(&recent, 100, 32);
    let frame = frame(&state, "fixture/model", "fixture-folder");
    let rows: Vec<_> = frame.live.iter().map(Line::plain).collect();
    assert_eq!(frame.live.len(), 32);
    let mark = rows.iter().position(|row| row.contains('▀')).unwrap();
    assert_eq!(mark, 3, "{rows:#?}");
    assert!(
        rows[mark..mark + 5]
            .iter()
            .all(|row| text::cells(row) <= 100)
    );
    assert_eq!(rows[mark + 5], "");
    assert_eq!(rows[mark + 6].trim(), VERSION);
    assert_eq!(rows[mark + 7], "");
    assert!(rows[mark + 8].contains("Tip  /permissions sets what Jecode may change here"));
    assert_eq!(rows[mark + 9].trim(), "✓ 7 skills   ✓ AGENTS.md");
    assert!(rows[mark + 11].contains("/  commands"));
    assert_eq!(rows[mark + 13].trim(), "Recent in this folder");
    assert!(rows[mark + 14].contains("Fix the session expiry check in auth"));
    assert!(rows[mark + 14].ends_with("2h"));
    assert!(rows[mark + 18].contains("/resume  all conversations"));
    assert!(rows[frame.composer - 1].is_empty());
    assert!(
        rows[frame.composer..]
            .iter()
            .any(|row| row.contains("fixture-folder"))
    );
    for fact in ["fixture-folder", "model · high"] {
        assert_eq!(rows.iter().filter(|row| row.contains(fact)).count(), 1);
    }
}

#[test]
fn scanline_mark_has_the_approved_width_ink_and_caret_timing() {
    let state = state(&[], 100, 32);
    motion::set_frame(0);
    let rows = opening::rows(&state, 100);
    let mark = rows
        .iter()
        .position(|row| row.plain().contains('▀'))
        .unwrap();
    for row in &rows[mark..mark + 5] {
        let text = row.plain();
        assert_eq!(text::cells(&text), 25 + 49);
        assert!(
            row.spans
                .iter()
                .any(|span| span.text.contains('▀') && span.style == ACCENT)
        );
    }
    assert!(
        rows[mark + 4]
            .spans
            .iter()
            .any(|span| span.text.contains('▀') && span.style == LIT)
    );
    motion::set_frame(4);
    let off = opening::rows(&state, 100);
    assert!(!off[mark + 4].spans.iter().any(|span| span.style == LIT));
    assert_eq!(off[mark].plain(), rows[mark].plain());
    motion::set_frame(8);
    assert_eq!(
        opening::rows(&state, 100)[mark + 4].plain(),
        rows[mark + 4].plain()
    );
    motion::set_frame(0);
}

#[test]
fn first_run_uses_real_facts_and_keeps_the_key_row() {
    let state = state(&[], 100, 32);
    let rows = opening::rows(&state, 100);
    let plain: Vec<_> = rows.iter().map(Line::plain).collect();
    assert!(
        plain
            .iter()
            .any(|row| row.contains("✓ 7 skills   – no AGENTS.md"))
    );
    assert!(plain.iter().any(|row| row.contains("Ctrl+Q  quit")));
    assert!(rows.iter().any(|row| {
        row.spans
            .iter()
            .any(|span| span.text == " Ctrl+Q " && span.background == Some(SELECTED_SURFACE))
    }));
    assert!(rows.iter().all(|row| !row.plain().contains("Recent")));
    let facts = rows
        .iter()
        .find(|row| row.plain().contains("7 skills"))
        .unwrap();
    assert!(
        facts
            .spans
            .iter()
            .any(|span| span.text == "✓ " && span.style == GOOD)
    );
    let tip = rows
        .iter()
        .find(|row| row.plain().contains("Tip "))
        .unwrap();
    assert!(
        tip.spans
            .iter()
            .any(|span| span.text == "Tip " && span.style == ACCENT.bold())
    );
    assert!(
        tip.spans
            .iter()
            .any(|span| span.text == " /permissions" && span.style == STRONG)
    );
    assert!(tip.spans.iter().any(|span| span.style == MUTED));
}

#[test]
fn viewport_matrix_keeps_every_row_inside_the_screen_and_follows_fallback_order() {
    let recent = [
        ("First conversation", "2h"),
        ("Second", "1d"),
        ("Third", "3d"),
    ];
    for columns in [48, 64, 80, 100, 120] {
        for height in [16, 24, 32, 40] {
            for saved in [false, true] {
                let state = state(if saved { &recent } else { &[] }, columns, height);
                let frame = frame(&state, "fixture/model", "fixture-folder");
                assert_eq!(frame.live.len(), height);
                assert!(
                    frame
                        .live
                        .iter()
                        .all(|line| text::cells(&line.plain()) <= columns)
                );
                let body = &frame.live[..frame.composer];
                let mark = body
                    .iter()
                    .any(|line| line.plain().contains(['▀', '▄', '█']));
                assert_eq!(
                    mark,
                    opening::animated(&state),
                    "{columns}x{height}, saved={saved}"
                );
                if columns < 53 {
                    assert!(body[0].plain().starts_with(">_ jecode"));
                }
                if saved {
                    assert!(
                        body.iter()
                            .any(|row| row.plain().contains("Recent in this folder"))
                    );
                } else {
                    assert!(body.iter().any(|row| row.plain().contains("/  commands")));
                }
            }
        }
    }
    let at = |height| opening::rows(&state(&recent, 80, height), 80);
    let full = at(40);
    assert!(full.iter().any(|row| row.plain().contains("Tip ")));
    assert!(full.iter().any(|row| row.plain().contains("Ctrl+O")));
    let shorter = at(24);
    assert!(!shorter.iter().any(|row| row.plain().contains("Tip ")));
    assert!(shorter.iter().any(|row| row.plain().contains("Recent")));
    let compact = at(20);
    assert!(compact.iter().any(|row| row.plain().contains('▄')));
    assert!(!compact.iter().any(|row| row.plain().contains("Ctrl+O")));
    let slim = at(16);
    assert!(slim[0].plain().starts_with(">_ jecode"));
}

#[test]
fn the_first_prompt_restores_the_slim_header_and_no_startup_facts() {
    let mut state = state(&[("Earlier work", "3d")], 100, 32);
    let mut layout = Layout::default();
    layout.prepare(&state, 100, 64);
    assert!(
        layout.blocks[0]
            .iter()
            .any(|row| row.plain().contains("Earlier work"))
    );
    state.message(Kind::User, "First prompt");
    layout.prepare(&state, 100, 64);
    assert_eq!(layout.blocks[0].len(), 2);
    assert_eq!(layout.blocks[0][0].plain(), format!(">_ jecode {VERSION}"));
    assert!(!opening::animated(&state));
    assert!(layout.blocks.iter().flatten().all(|row| {
        let text = row.plain();
        !text.contains("Earlier work") && !text.contains("7 skills")
    }));
}

#[test]
fn panels_and_a_tall_draft_reduce_the_actual_splash_room() {
    let mut state = state(&[("Earlier work", "3d")], 80, 24);
    assert!(opening::animated(&state));
    state.editor.insert(&"draft line\n".repeat(15));
    assert!(!opening::animated(&state));
    assert!(
        opening::rows(&state, 80)[0]
            .plain()
            .starts_with(">_ jecode")
    );
    state.editor = Default::default();
    state.activity = Some(Activity::new());
    state.quitting = Some(vec![("npm run dev".into(), "background process p3".into())]);
    let frame = frame(&state, "fixture/model", "fixture-folder");
    assert!(frame.cursor.is_none());
    assert!(
        frame.live[frame.composer..]
            .iter()
            .any(|row| row.plain().contains("Quit · work is running"))
    );
}

#[test]
fn a_startup_warning_keeps_the_splash_and_native_renderer_colours() {
    let mut state = state(&[], 100, 32);
    state.notice = Some(Feedback::result(Kind::Warning, "No model"));
    let frame = frame(&state, "", "fixture-folder");
    let rows: Vec<_> = frame.live.iter().map(Line::plain).collect();
    assert!(rows.iter().any(|row| row.contains("No model")));
    assert!(rows.iter().any(|row| row.contains("✓ 7 skills")));
    let output = Renderer::new().paint(&frame, 100, 32);
    assert!(output.contains("▀"));
    assert!(output.contains("No model"));
    assert!(output.contains(&format!("\x1b[{}m", ACCENT)));
    assert!(output.contains(&format!("\x1b[{}m", LIT)));
}
