//! Ask's answered-question step row and rendered Markdown task lists, read
//! back from the native console at 40 and 80 columns in both palettes. One
//! renderer paints every transition, so stale marks or titles would remain.
use super::work_tests::paint;
use crate::{
    ask::Request,
    json,
    tui::{ask::Panel, line::Line, markdown, render::Renderer, state::State, text, theme, view},
};

const CLEANUP: &str = r#"{"questions":[
    {"id":"scope","title":"Ambito della pulizia","question":"Cosa pulire?",
     "options":[{"id":"build","label":"Build"},{"id":"cache","label":"Cache"}]},
    {"id":"formats","title":"Formati","question":"Quali formati?","multiple":true,
     "options":[{"id":"csv","label":"CSV"},{"id":"xlsx","label":"XLSX"}]},
    {"id":"delivery","title":"Consegna","question":"Dove inviare?"}]}"#;

/// The step row starts with `prefix` and has `marks` circles, each followed by
/// one space and its title; the console then reads back the whole frame.
fn ask_frame(
    state: &State,
    renderer: &mut Renderer,
    size: (usize, usize),
    prefix: &str,
    marks: usize,
) {
    let frame = view::frame(state, "fixture/model", "fixture");
    let panel = state.ask_panel.as_ref().unwrap();
    let count = format!("{} of 3", panel.step + 1);
    let shown: Vec<String> = frame.live.iter().map(Line::plain).collect();
    let row = shown
        .iter()
        .find(|row| row.trim_end().ends_with(&count))
        .unwrap();
    assert!(row.starts_with(prefix), "{row:?} lacks {prefix:?}");
    assert_eq!(row.matches('◉').count(), marks, "{row:?}");
    assert!(
        !row.contains("◉…") && !row.contains("◉ …") && !row.contains('☑'),
        "{row:?}"
    );
    assert_eq!(text::cells(row.trim_end()), state.width - 2, "{row:?}");
    let output = paint(renderer, frame.live, size.0, size.1);
    if marks > 0 {
        assert!(output.contains(&format!(
            "\x1b[0m\x1b[{}m\x1b[{}m◉ ",
            theme::ACCENT,
            theme::BASE
        )));
    }
}

fn steps(renderer: &mut Renderer, size: (usize, usize), columns: usize) {
    let request = Request::parse("ask-marks", &json::parse(CLEANUP).unwrap()).unwrap();
    let [scope, formats, delivery] = [0, 1, 2].map(|index| request.questions[index].clone());
    let mut state = State {
        width: columns,
        height: size.1,
        question: Some(request.clone()),
        ask_panel: Some(Panel::new(&request, None)),
        ..State::default()
    };
    let wide = columns == 80;
    let first = "  Ambito della pulizia   ";
    ask_frame(&state, renderer, size, first, 0);
    let panel = state.ask_panel.as_mut().unwrap();
    panel.choose(&scope);
    panel.switch(&request, true);
    let second = if wide {
        "  ◉ Ambito della pulizia   Formati   Consegna"
    } else {
        "  …   Formati   Consegna"
    };
    ask_frame(&state, renderer, size, second, usize::from(wide));
    let panel = state.ask_panel.as_mut().unwrap();
    panel.select(&formats);
    panel.move_row(&formats, false);
    panel.select(&formats);
    panel.select(&formats);
    panel.switch(&request, true);
    let third = if wide {
        "  ◉ Ambito della pulizia   ◉ Formati   Consegna"
    } else {
        "  …   ◉ Formati   Consegna"
    };
    ask_frame(&state, renderer, size, third, 1 + usize::from(wide));
    let panel = state.ask_panel.as_mut().unwrap();
    panel.type_text(&delivery, "Via email");
    panel.switch(&request, false);
    let back = if wide {
        "  ◉ Ambito della pulizia   Formati   ◉ Consegna"
    } else {
        "  …   Formati   ◉ Consegna"
    };
    ask_frame(&state, renderer, size, back, 1 + usize::from(wide));
    // Clearing the formats removes their mark; reopening keeps the rest.
    let panel = state.ask_panel.as_mut().unwrap();
    panel.move_row(&formats, true);
    panel.select(&formats);
    panel.switch(&request, false);
    let draft = panel.draft(&request);
    state.ask_panel = Some(Panel::new(&request, Some(&draft)));
    let start = if wide {
        "  Ambito della pulizia   Formati   ◉ Consegna"
    } else {
        first
    };
    ask_frame(&state, renderer, size, start, usize::from(wide));
}

/// Task lists switch states and shrink between frames.
fn tasks(renderer: &mut Renderer, size: (usize, usize), columns: usize) {
    let stages: [(&str, &[&str]); 3] = [
        (
            "- [ ] Pulire i file temporanei della build\n- [ ] Ambito della pulizia\n  - [ ] Cache",
            &["○ Pulire i file", "○ Ambito della", "  ○ Cache"],
        ),
        (
            "- [x] Pulire i file temporanei della build\n- [ ] Ambito della pulizia\n  - [X] Cache",
            &["◉ Pulire i file", "○ Ambito della", "  ◉ Cache"],
        ),
        ("- [x] Pulire\n- [x] Ambito", &["◉ Pulire", "◉ Ambito"]),
    ];
    for (source, expected) in stages {
        let rows = markdown::render(source, columns);
        let shown: Vec<String> = rows.iter().map(Line::plain).collect();
        for prefix in expected {
            assert!(
                shown.iter().any(|row| row.starts_with(prefix)),
                "{prefix}: {shown:#?}"
            );
        }
        // Wrapped words hang under the title, two cells past the mark.
        for (row, next) in shown.iter().zip(shown.iter().skip(1)) {
            if row.starts_with(['○', '◉']) && !next.trim_start().starts_with(['○', '◉']) {
                assert!(
                    next.starts_with("  ") && !next[2..].starts_with(' '),
                    "{next:?}"
                );
            }
        }
        assert!(!shown.iter().any(|row| row.contains(['☐', '☑'])));
        let output = paint(renderer, rows, size.0, size.1);
        // Unchanged rows keep their native cells and emit no VT. Check inks
        // for repainted marks; readback still checks the whole frame.
        for (mark, style) in [('○', theme::MUTED), ('◉', theme::GOOD)] {
            if output.contains(mark) {
                assert!(
                    output.contains(&format!("\x1b[0m\x1b[{style}m\x1b[{}m{mark} ", theme::BASE)),
                    "{mark} did not carry {style} in emitted VT"
                );
            }
        }
    }
}

pub(super) fn console(renderer: &mut Renderer, width: usize, height: usize) {
    for columns in [40, 80] {
        for light in [false, true] {
            theme::set_light(light);
            renderer.repaint();
            steps(renderer, (width, height), columns);
            tasks(renderer, (width, height), columns);
        }
    }
    theme::set_light(false);
    renderer.repaint();
}
