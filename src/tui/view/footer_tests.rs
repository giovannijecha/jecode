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
            tokens: Some(tokens),
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
fn the_footer_shows_the_latest_provider_usage() {
    let mut state = with_status(61_000, Some(100_000));
    let idle = frame(&state, "provider/model", r"C:\project");
    assert_eq!(idle.live.len(), 5);
    let line = idle.live.last().unwrap();
    assert!(line.plain().starts_with(r"C:\project"));
    // 61% of an 80% threshold fills six of eight cells.
    assert!(
        line.plain()
            .ends_with(" provider/model · default   ▰▰▰▰▰▰▱▱ 61%")
    );
    let styles = |line: &Line, glyph: &str| -> Vec<_> {
        let spans = line.spans.iter().filter(|span| span.text.contains(glyph));
        spans.map(|span| span.style).collect()
    };
    assert_eq!(styles(line, "▰"), [METER_FILL]);
    assert_eq!(styles(line, "▱▱"), [METER_TRACK]);
    state.activity = Some(Activity::new());
    assert!(footer(&state).contains("▰▰▰▰▰▰▱▱ 61%"));
    assert!(footer(&with_status(5, Some(1_000_000))).contains("▱▱▱▱▱▱▱▱ <1%"));
    // Without an engine status the footer keeps the model chip.
    assert!(footer(&State::default()).ends_with("  provider/model · default "));
}

#[test]
fn the_meter_turns_gold_near_the_threshold_and_stays_empty_when_unknown() {
    let state = with_status(78_000, Some(100_000));
    let near = frame(&state, "provider/model", r"C:\project");
    let line = near.live.last().unwrap();
    assert!(line.plain().contains("▰▰▰▰▰▰▰▰ 78%"));
    let warning = line.spans.iter().filter(|span| span.style == WARNING);
    assert_eq!(
        warning.map(|span| span.text.trim()).collect::<Vec<_>>(),
        ["▰▰▰▰▰▰▰▰ 78%"]
    );
    let mut unknown = with_status(5000, None);
    for width in [80, 30, 12] {
        unknown.width = width;
        let line = footer(&unknown);
        assert!(line.contains("▱▱▱▱▱▱▱▱ ?"), "{width}: {line}");
        assert!(!line.contains('%') && text::cells(&line) <= width);
    }
    unknown.context.as_mut().unwrap().capacity = Some(100_000);
    unknown.context.as_mut().unwrap().tokens = None;
    for width in [80, 30, 12] {
        unknown.width = width;
        let line = footer(&unknown);
        assert!(line.contains("▱▱▱▱▱▱▱▱ ?"), "{width}: {line}");
        assert!(!line.contains('%') && text::cells(&line) <= width);
    }
}

#[test]
fn a_low_auto_compact_threshold_is_not_warned_at_zero_percent() {
    let mut state = with_status(0, Some(100_000));
    for threshold in 1..=3 {
        state.context.as_mut().unwrap().auto_compact = threshold;
        let idle = frame(&state, "provider/model", r"C:\project");
        let line = idle.live.last().unwrap();
        assert!(line.spans.iter().all(|span| span.style != WARNING));
    }
    state.context.as_mut().unwrap().tokens = Some(1_000);
    let active = frame(&state, "provider/model", r"C:\project");
    assert!(
        active
            .live
            .last()
            .unwrap()
            .spans
            .iter()
            .any(|span| span.style == WARNING)
    );
}

#[test]
fn narrow_footers_shorten_the_directory_and_model_before_the_meter() {
    let mut state = with_status(61_000, Some(100_000));
    let full = " provider/model · default   ▰▰▰▰▰▰▱▱ 61%";
    let detailed = " model · default   ▰▰▰▰▰▰▱▱ 61%";
    let short = " model   ▰▰▰▰▰▰▱▱ 61%";
    for (width, left, right) in [
        (52, r"C:\project", full),
        (51, r"C:\project", detailed),
        (43, r"C:\project", detailed),
        (42, "project", detailed),
        (39, "project", short),
        (29, "", short),
        (20, "", "▰▰▰▰▰▰▱▱ 61%"),
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
fn open_panels_replace_the_footer_on_the_terminal_surface() {
    // At twelve rows the settings body shrinks to three options.
    for (menu, keys, short) in [
        (
            Selector::settings(
                "provider/model",
                Effort::Default,
                80,
                &crate::backend::openrouter::Routing::Auto,
                Default::default(),
                std::path::Path::new("fixture/config.json"),
            ),
            "   ↑↓  move    Enter  change    Esc  close",
            "Default routing",
        ),
        (
            Selector::auto_compact(80),
            "   Enter  save    Esc  back",
            "Auto-compact at",
        ),
    ] {
        let mut state = with_status(61_000, Some(100_000));
        let idle = footer(&state);
        assert!(idle.ends_with(" provider/model · default   ▰▰▰▰▰▰▱▱ 61%"));
        state.selector = Some(menu);
        for height in [24, 12] {
            state.height = height;
            let open = frame(&state, "provider/model", r"C:\project");
            let rows: Vec<_> = open.live.iter().map(|line| line.plain()).collect();
            // The panel's keys take the footer's place as the last row.
            assert_eq!(rows.last().unwrap().trim_end(), keys, "{rows:#?}");
            assert!(!rows.iter().any(|row| row.contains("61%") || *row == idle));
            assert!(open.live.iter().all(|line| line.background.is_none()));
            let shown = if height == 12 {
                short
            } else {
                "Auto-compact at"
            };
            assert!(rows.iter().any(|row| row.contains(shown)), "{rows:#?}");
        }
    }
    let mut state = with_status(61_000, Some(100_000));
    state.selector = Some(Selector::auto_compact(80));
    let open = frame(&state, "provider/model", r"C:\project");
    let rows = open
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>();
    // The value is the composer's field with the caret after it; a form
    // without body rows has no blank row before its keys.
    let (row, column) = open.cursor.unwrap();
    assert_eq!(rows[row], "› 80% of the context window");
    assert_eq!(column, 4);
    assert!(rows[row - 1].starts_with("─ Threshold ─"));
    assert!(rows[row + 1].starts_with("─ Settings › Auto-compact at "));
    assert!(rows[row + 1].ends_with(" this and new conversations ─"));
    assert_eq!(rows[row + 2], "   Enter  save    Esc  back");
    assert_eq!(rows.len(), row + 3);
    assert!(rows.iter().all(|row| text::cells(row) <= state.width));
}
