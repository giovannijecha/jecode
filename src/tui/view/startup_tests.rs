use super::*;
use crate::tui::{
    activity::Activity,
    feedback::Feedback,
    motion,
    render::Renderer,
    state::{Kind, Opening, Recent},
    text,
    theme::{ACCENT, DIM, GOOD, LIT, MUTED, STRONG},
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
            needs_model: recent.is_empty(),
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
    assert_eq!(mark, 6, "{rows:#?}");
    assert!(
        rows[mark..mark + 5]
            .iter()
            .all(|row| text::cells(row) <= 100)
    );
    assert!(rows[mark + 5].trim().is_empty());
    // The version at the mark's left edge, the facts at its right, one block of 49 cells.
    let facts = rows[mark + 6].trim();
    assert!(
        facts.starts_with(VERSION) && facts.ends_with("✓ 7 skills   ✓ AGENTS.md"),
        "{rows:#?}"
    );
    assert!(rows[mark + 7].trim().is_empty());
    assert!(rows[mark + 8].trim().starts_with("Recent in this folder"));
    assert!(rows[mark + 8].trim().ends_with("/resume all"));
    assert!(rows[mark + 9].contains("Fix the session expiry check in auth"));
    assert!(rows[mark + 9].ends_with("2h"));
    assert!(rows[mark + 11].contains("Migrate the config loader"));
    assert!(rows[mark + 12].trim().is_empty());
    let image_key = if cfg!(target_os = "macos") {
        "Ctrl+V"
    } else {
        "Alt+V"
    };
    assert_eq!(
        rows[mark + 13].trim(),
        format!("/ commands   {image_key} image   Ctrl+Q quit")
    );
    assert!(
        rows.iter()
            .all(|row| !row.contains("Tip") && !row.contains("Ctrl+O"))
    );
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
    assert!(plain.iter().any(|row| row.contains("Ctrl+Q quit")));
    // Keys are bold text without a surface; a first run has no Ctrl+O and no tip.
    assert!(rows.iter().any(|row| {
        row.spans
            .iter()
            .any(|span| span.text == "Ctrl+Q" && span.style == STRONG.bold())
    }));
    assert!(
        rows.iter()
            .flat_map(|row| &row.spans)
            .all(|span| span.background.is_none())
    );
    assert!(
        rows.iter()
            .all(|row| !row.plain().contains("Recent") && !row.plain().contains("Ctrl+O"))
    );
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
    // Start here: three commands in STRONG, each with a MUTED fact.
    let start = rows
        .iter()
        .position(|row| row.plain().contains("Start here"))
        .unwrap();
    assert!(
        rows[start]
            .spans
            .iter()
            .any(|span| span.text == "Start here" && span.style == DIM)
    );
    for (offset, command) in ["/model", "/permissions", "/help"].into_iter().enumerate() {
        let row = &rows[start + 1 + offset];
        assert!(
            row.plain().trim_start().starts_with(command),
            "{}",
            row.plain()
        );
        assert!(
            row.spans
                .iter()
                .any(|span| span.text.trim() == command && span.style == STRONG)
        );
        assert!(row.spans.iter().any(|span| span.style == MUTED));
    }
    assert!(rows.iter().all(|row| !row.plain().contains("Tip ")));
}

#[test]
fn a_selected_model_without_local_conversations_does_not_show_setup_advice() {
    use crate::{
        agent::Agent,
        backend::unconfigured,
        config::{Settings, Store},
        effort::Effort,
        session::SessionConfig,
        test_support::{Directory, HttpFixture},
        tools::Tools,
        tui::App,
    };

    for model in [None, Some("saved-model")] {
        let directory = Directory::new();
        let settings = model.map_or_else(Settings::unconfigured, |model| {
            Settings::selected("openai", String::new(), model.into()).unwrap()
        });
        let agent = Agent::with_backend(
            unconfigured::client(
                &settings.backend_id,
                model.map(str::to_owned),
                None,
                "No configured provider".into(),
            ),
            Tools::new(directory.path()).unwrap(),
        );
        let app = App::new(
            agent,
            SessionConfig {
                store: Store::new(directory.path().join("config")),
                settings,
            },
            None,
        );
        assert!(app.state.opening.recent.is_empty());
        let rows = opening::rows(&app.state, 100);
        let text: Vec<_> = rows.iter().map(Line::plain).collect();
        assert_eq!(app.state.opening.needs_model, model.is_none());
        assert_eq!(
            text.iter().any(|row| row.contains("Start here")),
            model.is_none()
        );
        assert_eq!(
            text.iter()
                .any(|row| row.contains("No saved conversations in this folder")),
            model.is_some()
        );
    }

    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let services = crate::backend::Services::fixture(&fixture.endpoint);
    let agent = Agent::with_backend(
        unconfigured::client(
            "openrouter",
            None,
            Some(services.clone()),
            "Choose a model".into(),
        ),
        Tools::new(directory.path()).unwrap(),
    );
    let mut app = App::new(
        agent,
        SessionConfig {
            store: Store::new(directory.path().join("config")),
            settings: Settings::unconfigured(),
        },
        None,
    );
    assert!(app.state.opening.needs_model);
    app.catalog_services = Some(services);
    app.apply_selection("fixture/model".into(), Effort::Default, false);
    assert!(app.config.settings.has_model());
    assert!(app.agent.as_ref().unwrap().has_model());
    assert!(!app.state.opening.needs_model);
    let rows = opening::rows(&app.state, 100);
    assert!(rows.iter().any(|row| {
        row.plain()
            .contains("No saved conversations in this folder")
    }));
    assert!(rows.iter().all(|row| !row.plain().contains("Start here")));
    assert!(fixture.finish().is_empty());
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
                    assert!(body.iter().any(|row| row.plain().contains("Start here")));
                }
            }
        }
    }
    let at = |height| opening::rows(&state(&recent, 80, height), 80);
    // With room the keys close the block; short windows drop them, then compact the mark.
    let keys = |rows: &[Line]| rows.iter().any(|row| row.plain().contains("/ commands"));
    let full = at(40);
    assert!(keys(&full));
    assert!(full.iter().all(|row| !row.plain().contains("Tip ")));
    assert!(full.iter().all(|row| !row.plain().contains("Ctrl+O")));
    let shorter = at(24);
    assert!(keys(&shorter));
    assert!(shorter.iter().any(|row| row.plain().contains("Recent")));
    let no_keys = at(18);
    assert!(!keys(&no_keys));
    assert!(no_keys.iter().any(|row| row.plain().contains("Recent")));
    assert!(no_keys.iter().any(|row| row.plain().contains('▀')));
    let compact = at(16);
    assert!(compact.iter().any(|row| row.plain().contains('▄')));
    assert!(!keys(&compact));
    let slim = at(14);
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
    let mut state = state(&[("Earlier work", "3d")], 80, 18);
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
