use super::composer_frame as frame;
use super::*;
use crate::{
    context::Status,
    effort::Effort,
    tui::{
        activity::Activity,
        line::Line,
        selector::Selector,
        theme::{METER_FILL, METER_TRACK, WARNING},
    },
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
fn the_footer_meters_the_projection_toward_auto_compaction() {
    let mut state = with_status(61_000, Some(100_000));
    let idle = frame(&state, "provider/model", r"C:\project");
    assert_eq!(idle.live.len(), 4);
    let line = idle.live.last().unwrap();
    assert!(line.plain().starts_with(r"C:\project"));
    // 61% of an 80% threshold fills six of eight cells.
    assert!(
        line.plain()
            .ends_with("━━━━━━── 61% · provider/model · default")
    );
    let styles = |line: &Line, glyph: &str| -> Vec<_> {
        let spans = line.spans.iter().filter(|span| span.text.contains(glyph));
        spans.map(|span| span.style).collect()
    };
    assert_eq!(styles(line, "━"), [METER_FILL]);
    assert_eq!(styles(line, "──"), [METER_TRACK]);
    state.activity = Some(Activity::new());
    assert!(footer(&state).contains("━━━━━━── 61%"));
    assert!(footer(&with_status(5, Some(1_000_000))).contains("──────── <1%"));
    // Without an engine status the footer keeps the model and effort.
    assert!(footer(&State::default()).ends_with("  provider/model · default"));
}

#[test]
fn the_meter_turns_gold_near_the_threshold_and_stays_empty_when_unknown() {
    let state = with_status(78_000, Some(100_000));
    let near = frame(&state, "provider/model", r"C:\project");
    let line = near.live.last().unwrap();
    assert!(line.plain().contains("━━━━━━━━ 78%"));
    let warning = line.spans.iter().filter(|span| span.style == WARNING);
    assert_eq!(
        warning.map(|span| span.text.trim()).collect::<Vec<_>>(),
        ["━━━━━━━━ 78%"]
    );
    let mut unknown = with_status(5000, None);
    for width in [80, 30, 12] {
        unknown.width = width;
        let line = footer(&unknown);
        assert!(line.contains("──────── ?"), "{width}: {line}");
        assert!(!line.contains('%') && text::cells(&line) <= width);
    }
}

#[test]
fn narrow_footers_shorten_the_directory_and_model_before_the_meter() {
    let mut state = with_status(61_000, Some(100_000));
    for (width, left, right) in [
        (51, r"C:\project", "━━━━━━── 61% · provider/model · default"),
        (50, "project", "━━━━━━── 61% · model · default"),
        (38, "", "━━━━━━── 61% · model"),
        (19, "", "━━━━━━── 61%"),
    ] {
        state.width = width;
        let gap = " ".repeat(width - text::cells(left) - text::cells(right));
        assert_eq!(footer(&state), format!("{left}{gap}{right}"), "{width}");
    }
    state.width = 7;
    assert!(text::cells(&footer(&state)) <= 7);
    // The extended-length prefix never reaches the footer.
    state.width = 80;
    let frame = frame(&state, "provider/model", r"\\?\C:\project");
    assert!(
        frame
            .live
            .last()
            .unwrap()
            .plain()
            .starts_with(r"C:\project  ")
    );
}

#[test]
fn the_directory_reads_relative_to_home() {
    let home = Some(r"C:\Users\me");
    assert_eq!(
        footer::home_relative(r"C:\Users\me\Projects\app", home),
        r"~\Projects\app"
    );
    assert_eq!(footer::home_relative(r"C:\Users\me", home), "~");
    assert_eq!(
        footer::home_relative(r"C:\Users\meg\app", home),
        r"C:\Users\meg\app"
    );
    assert_eq!(footer::home_relative(r"D:\work", home), r"D:\work");
    assert_eq!(
        footer::home_relative("/home/me/app", Some("/home/me/")),
        "~/app"
    );
    assert_eq!(footer::home_relative(r"C:\app", None), r"C:\app");
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
    assert!(frame.live.iter().all(|line| !line.plain().contains("61%")));
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
        assert!(last.plain().contains("━━━━━━── 61% · provider"));
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
    assert!(
        rows.iter().any(|row| row.contains("less room for")),
        "{rows:#?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains(" Enter  save    Esc  back"))
    );
    assert!(text::cells(&rows[rows.len() - 1]) <= state.width);
}
