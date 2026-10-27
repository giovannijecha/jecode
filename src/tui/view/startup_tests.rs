use super::*;
use crate::tui::{
    activity::Activity,
    state::{Kind, Opening, Recent},
    text,
    theme::{ACCENT, BODY, DIM, MUTED, SELECTED_SURFACE, STRONG},
    transcript::Layout,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn state(recent: &[(&str, &str)]) -> State {
    State {
        effort: "high".into(),
        opening: Opening {
            recent: recent
                .iter()
                .map(|(title, age)| Recent {
                    title: (*title).into(),
                    age: (*age).into(),
                })
                .collect(),
        },
        ..State::default()
    }
}

fn styles(line: &Line) -> Vec<(String, crate::tui::theme::Style)> {
    line.spans
        .iter()
        .map(|span| (span.text.clone(), span.style))
        .collect()
}

#[test]
fn the_header_names_the_product_and_version_and_leaves_the_rest_to_the_footer() {
    let rows = opening::rows(&state(&[]), 80);
    assert_eq!(rows[0].plain(), format!(">_ jecode {VERSION}"));
    assert_eq!(
        styles(&rows[0]),
        [
            (">_".into(), ACCENT.bold()),
            (" jecode".into(), STRONG.bold()),
            (format!(" {VERSION}"), MUTED)
        ]
    );
    assert_eq!(rows[1], Line::default());
    for columns in [2, 12, 30] {
        for row in &opening::rows(&state(&[("A title", "2h")]), columns) {
            assert!(text::cells(&row.plain()) <= columns, "{columns}");
        }
    }
    // Model, effort and folder appear once on screen, in the footer.
    let frame = frame(&state(&[]), "fixture/model", "fixture-folder");
    let screen: Vec<_> = frame.live.iter().map(Line::plain).collect();
    for fact in ["fixture-folder", "model · high"] {
        let shown = screen.iter().filter(|row| row.contains(fact)).count();
        assert_eq!(shown, 1, "{fact}: {screen:#?}");
    }
}

#[test]
fn the_first_run_offers_one_key_row_that_drops_keys_from_the_right() {
    let rows = opening::rows(&state(&[]), 80);
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[2].plain(),
        "   /  commands    Alt+V  image    Ctrl+O  expand    Ctrl+Q  quit"
    );
    assert!(
        rows[2]
            .spans
            .iter()
            .any(|span| span.text == " Ctrl+Q " && span.background == Some(SELECTED_SURFACE))
    );
    let narrow = opening::rows(&state(&[]), 40)[2].plain();
    assert_eq!(narrow, "   /  commands    Alt+V  image");
}

#[test]
fn recent_conversations_end_their_age_two_cells_from_the_edge_then_offer_resume() {
    let recent = [
        ("Fix the session expiry check in auth", "2h"),
        ("Add the sleep test", "1d"),
    ];
    let rows = opening::rows(&state(&recent), 60);
    let plain: Vec<_> = rows.iter().map(Line::plain).collect();
    assert_eq!(plain[2], "  Recent in this folder");
    assert_eq!(rows[2].spans.last().unwrap().style, DIM);
    assert!(plain[3].starts_with("  Fix the session expiry check in auth "));
    assert!(plain[3].ends_with(" 2h"));
    assert_eq!(text::cells(&plain[3]), 58);
    assert!(plain[4].starts_with("  Add the sleep test ") && plain[4].ends_with(" 1d"));
    let title = rows[3].spans.iter().find(|span| span.text.contains("Fix"));
    assert_eq!(title.unwrap().style, BODY);
    assert_eq!(rows[3].spans.last().unwrap().style, MUTED);
    assert_eq!(plain[5], "");
    assert_eq!(plain[6], "   /resume  all conversations");
    assert_eq!(plain.len(), 7);
}

#[test]
fn the_startup_space_leaves_with_the_first_prompt() {
    let mut state = state(&[("Earlier work", "3d")]);
    let mut layout = Layout::default();
    layout.prepare(&state, 60, 64);
    assert_eq!(layout.blocks[0].len(), 6);
    state.message(Kind::User, "First prompt");
    layout.prepare(&state, 60, 64);
    assert_eq!(layout.blocks[0].len(), 2);
    assert!(
        layout
            .blocks
            .iter()
            .flatten()
            .all(|row| !row.plain().contains("Earlier work"))
    );
}

#[test]
fn the_quit_guard_lists_the_turn_and_processes_and_asks_in_place_of_the_keys() {
    let mut state = state(&[]);
    state.activity = Some(Activity::new());
    state.quitting = Some(vec![("npm run dev".into(), "background process p3".into())]);
    let frame = frame(&state, "fixture/model", "fixture");
    let dock: Vec<_> = frame.live[frame.composer..]
        .iter()
        .map(Line::plain)
        .collect();
    assert!(
        dock.iter()
            .any(|row| row.starts_with("─ Quit · work is running"))
    );
    assert!(
        dock.iter()
            .any(|row| row.starts_with("  Current turn  Working · "))
    );
    assert!(
        dock.iter()
            .any(|row| row.starts_with("  npm run dev   background process p3"))
    );
    let question = dock.last().unwrap();
    assert!(
        question.starts_with("  Stop both and quit?   "),
        "{question}"
    );
    assert!(question.contains(" Enter  quit   ") && question.contains(" Esc  cancel"));
    assert!(frame.cursor.is_none());
    assert!(panel_open(&state));
}
