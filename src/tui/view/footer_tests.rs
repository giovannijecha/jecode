use super::composer_frame as frame;
use super::*;
use crate::{
    context::Status,
    effort::Effort,
    tui::{activity::Activity, selector::Selector, theme::MUTED},
};

fn with_status(tokens: usize, capacity: Option<usize>) -> State {
    State {
        context: Some(Status {
            tokens,
            capacity,
            auto_compact: 80,
        }),
        ..State::default()
    }
}

fn footer(state: &State) -> String {
    let frame = frame(state, "provider/model", r"C:\project");
    frame.live.last().unwrap().plain()
}

#[test]
fn the_footer_reports_the_projection_idle_and_while_working() {
    let mut state = with_status(61_000, Some(100_000));
    let idle = frame(&state, "provider/model", r"C:\project");
    assert_eq!(idle.live.len(), 4);
    let line = idle.live.last().unwrap();
    assert!(line.plain().starts_with(r"C:\project"));
    assert!(
        line.plain()
            .ends_with("Context ≈61% · Auto 80% · provider/model · default")
    );
    assert!(line.spans.iter().all(|span| span.style == MUTED));
    state.activity = Some(Activity::new());
    assert!(footer(&state).contains("Context ≈61% · Auto 80%"));
    let tiny = with_status(5, Some(1_000_000));
    assert!(footer(&tiny).contains("Context <1% · Auto 80%"));
    // Without an engine status the footer keeps its previous form.
    assert!(footer(&State::default()).ends_with("  provider/model · default"));
}

#[test]
fn unknown_capacity_is_explicit_and_never_a_percentage() {
    let mut state = with_status(5000, None);
    for width in [80, 60, 50, 30, 20] {
        state.width = width;
        let line = footer(&state);
        assert!(line.contains("Context"), "{width}: {line}");
        assert!(!line.contains('≈') && !line.contains("<1%"));
        assert!(text::cells(&line) <= width);
    }
    state.width = 80;
    assert!(footer(&state).contains("Context unknown · Auto 80%"));
}

#[test]
fn narrow_footers_abbreviate_the_indicator_before_dropping_it() {
    let mut state = with_status(61_000, Some(100_000));
    for (width, expected, hidden) in [
        (62, "Context ≈61% · Auto 80% · provider", ""),
        (61, "Context ≈61% · provider", "Auto"),
        (43, "≈61% · provider", "Context"),
        (20, "≈61% · provider", "Context"),
        (12, "≈61%", "Context"),
    ] {
        state.width = width;
        let line = footer(&state);
        assert!(line.contains(expected), "{width}: {line}");
        assert!(
            hidden.is_empty() || !line.contains(hidden),
            "{width}: {line}"
        );
        assert!(text::cells(&line) <= width);
    }
    state.width = 7;
    assert!(text::cells(&footer(&state)) <= 7);
    // A long model omits the directory instead of leaving a lone ellipsis.
    state.width = 46;
    let frame = frame(&state, "anthropic/claude-sonnet-5.5", r"\\?\C:\project");
    let line = frame.live.last().unwrap().plain();
    assert!(line.trim_start().starts_with("≈61% · anthropic"), "{line}");
    assert!(!line.contains('…') && text::cells(&line) <= 46, "{line}");
    state.width = 80;
    assert!(footer(&state).starts_with(r"C:\project  "));
}

#[test]
fn short_viewports_keep_the_input_and_caret_before_the_footer() {
    let mut state = with_status(61_000, Some(100_000));
    state.height = 3;
    state.editor.insert("draft");
    let frame = frame(&state, "provider/model", r"C:\project");
    assert_eq!(frame.live.len(), 3);
    let (row, _) = frame.cursor.unwrap();
    assert!(frame.live[row].plain().contains("draft"));
    assert!(
        frame
            .live
            .iter()
            .all(|line| !line.plain().contains("Context"))
    );
}

#[test]
fn open_panels_keep_the_footer_on_the_terminal_surface_when_space_permits() {
    for menu in [
        Selector::settings(
            "provider/model",
            Effort::Default,
            80,
            &crate::backend::openrouter::Routing::Auto,
        ),
        Selector::auto_compact(80),
    ] {
        let mut state = with_status(61_000, Some(100_000));
        state.selector = Some(menu);
        let open = frame(&state, "provider/model", r"C:\project");
        let (last, panel) = open.live.split_last().unwrap();
        assert!(last.plain().contains("Context ≈61% · Auto 80% · provider"));
        assert_eq!(last.background, None);
        assert!(panel.iter().all(|line| line.background.is_some()));
        assert!(
            panel
                .iter()
                .any(|line| line.plain().contains("Auto-compact at"))
        );
        state.height = 12;
        let short = frame(&state, "provider/model", r"C:\project");
        assert!(short.live.iter().all(|line| line.background.is_some()));
        assert!(
            short
                .live
                .iter()
                .any(|line| line.plain().contains("Auto-compact at"))
        );
    }
    let mut state = with_status(61_000, Some(100_000));
    state.selector = Some(Selector::auto_compact(80));
    let open = frame(&state, "provider/model", r"C:\project");
    let rows = open
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>();
    let (row, _) = open.cursor.unwrap();
    assert!(rows[row].contains("80"));
    assert!(rows.iter().any(|row| row.contains("less room for output")));
    assert!(
        rows.iter()
            .any(|row| row.contains("Enter save · Esc close"))
    );
    assert!(text::cells(&rows[rows.len() - 1]) <= state.width);
}
