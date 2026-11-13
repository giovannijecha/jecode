//! The step row over several questions: `◉ ` before each answered title other
//! than the current one, through answering, switching, changing answers and
//! reopening, at every width the row must fit.
use super::Panel;
use crate::{
    ask::Request,
    json,
    tui::{line::Line, state::State, text, theme::ACCENT, view},
};

/// Single choice, multiple choice and typed words, as in the reported screen.
const CLEANUP: &str = r#"{"questions":[
    {"id":"scope","title":"Ambito della pulizia","question":"Cosa pulire?",
     "options":[{"id":"build","label":"Build"},{"id":"cache","label":"Cache"}]},
    {"id":"formats","title":"Formati","question":"Quali formati?","multiple":true,
     "options":[{"id":"csv","label":"CSV"},{"id":"xlsx","label":"XLSX"}]},
    {"id":"delivery","title":"Consegna","question":"Dove inviare?"}]}"#;

fn request(arguments: &str) -> Request {
    Request::parse("call", &json::parse(arguments).unwrap()).unwrap()
}

fn dock(request: &Request, panel: &Panel, width: usize) -> Vec<Line> {
    let state = State {
        width,
        height: 30,
        question: Some(request.clone()),
        ask_panel: Some(panel.clone()),
        ..State::default()
    };
    view::composer_frame(&state, "fixture", "folder").live
}

/// The step row, checked for what every frame shares: `N of M` ends two cells
/// from the edge, the current title is bold accent and unmarked, and each mark
/// is accent, followed by one space and a title, never a cut.
fn steps(request: &Request, panel: &Panel, width: usize) -> Line {
    let rows = dock(request, panel, width);
    let count = format!("{} of {}", panel.step + 1, request.questions.len());
    let row = rows
        .iter()
        .find(|row| row.plain().trim_end().ends_with(&count))
        .unwrap_or_else(|| {
            panic!(
                "{width}: {:?}",
                rows.iter().map(Line::plain).collect::<Vec<_>>()
            )
        })
        .clone();
    let plain = row.plain();
    assert_eq!(text::cells(plain.trim_end()), width - 2, "{plain:?}");
    assert!(!plain.contains(['☐', '☑']), "{plain:?}");
    let current = text::clean(&request.questions[panel.step].title);
    // The current title takes priority over earlier titles and their prefix;
    // reserve a trailing ellipsis if later questions remain.
    let later = usize::from(panel.step + 1 < request.questions.len());
    if text::cells(&current) + text::cells(&count) + 6 + later <= width {
        let span = row
            .spans
            .iter()
            .find(|span| span.text.starts_with(&current));
        assert_eq!(
            span.map(|span| span.style),
            Some(ACCENT.bold()),
            "{plain:?}"
        );
    }
    let characters: Vec<char> = plain.chars().collect();
    for (index, _) in characters.iter().enumerate().filter(|(_, c)| **c == '◉') {
        let next = characters.get(index + 1..index + 3);
        assert!(
            matches!(next, Some([' ', title]) if !matches!(title, ' ' | '…')),
            "{width}: {plain:?}"
        );
    }
    for span in row.spans.iter().filter(|span| span.text.contains('◉')) {
        assert_eq!((span.text.as_str(), span.style), ("◉ ", ACCENT));
    }
    row
}

fn starts(request: &Request, panel: &Panel, width: usize, prefix: &str, marks: usize) {
    let plain = steps(request, panel, width).plain();
    assert!(
        plain.starts_with(prefix),
        "{width}: {plain:?} lacks {prefix:?}"
    );
    assert_eq!(plain.matches('◉').count(), marks, "{width}: {plain:?}");
}

fn options(request: &Request, panel: &Panel, width: usize, expected: &[&str]) {
    let rows: Vec<String> = dock(request, panel, width)
        .iter()
        .map(Line::plain)
        .collect();
    for prefix in expected {
        assert!(
            rows.iter().any(|row| row.starts_with(prefix)),
            "{prefix}: {rows:#?}"
        );
    }
}

#[test]
fn answered_titles_other_than_the_current_one_carry_a_filled_circle() {
    let request = request(CLEANUP);
    let [scope, formats, delivery] = [0, 1, 2].map(|index| &request.questions[index]);
    for width in [40, 80] {
        let wide = width == 80;
        let mut panel = Panel::new(&request, None);
        let first = "  Ambito della pulizia   ";
        starts(&request, &panel, width, first, 0);

        // Answering the current question marks nothing until it is left.
        assert!(panel.choose(scope));
        starts(&request, &panel, width, first, 0);
        assert!(panel.switch(&request, true));
        let second = if wide {
            "  ◉ Ambito della pulizia   Formati   Consegna"
        } else {
            "  …   Formati   Consegna"
        };
        starts(&request, &panel, width, second, usize::from(wide));

        // Multiple choice: focus moves and toggles without following selection.
        panel.select(formats);
        panel.move_row(formats, false);
        panel.select(formats);
        panel.select(formats);
        options(
            &request,
            &panel,
            width,
            &["  ◉ CSV", "› ○ XLSX", "  ○ Other"],
        );
        assert_eq!(panel.entry().answer(formats).unwrap().choices, ["csv"]);
        starts(&request, &panel, width, second, usize::from(wide));
        assert!(panel.switch(&request, true));
        let third = if wide {
            "  ◉ Ambito della pulizia   ◉ Formati   Consegna"
        } else {
            "  …   ◉ Formati   Consegna"
        };
        starts(&request, &panel, width, third, 1 + usize::from(wide));

        // Typed words answer the free question; moving back marks it.
        panel.type_text(delivery, "Via email");
        assert!(panel.switch(&request, false));
        let back = if wide {
            "  ◉ Ambito della pulizia   Formati   ◉ Consegna"
        } else {
            "  …   Formati   ◉ Consegna"
        };
        starts(&request, &panel, width, back, 1 + usize::from(wide));

        // Clearing an answer removes its mark; changing one keeps it.
        panel.move_row(formats, true);
        panel.select(formats);
        options(&request, &panel, width, &["› ○ CSV", "  ○ XLSX"]);
        assert!(panel.switch(&request, false));
        panel.move_row(scope, false);
        assert!(panel.choose(scope));
        assert_eq!(panel.entry().answer(scope).unwrap().choices, ["cache"]);
        let marks = usize::from(wide);
        let wide_row = "  Ambito della pulizia   Formati   ◉ Consegna";
        starts(
            &request,
            &panel,
            width,
            if wide { wide_row } else { first },
            marks,
        );

        // Reopening restores the answers, so the same row returns.
        let shown = steps(&request, &panel, width).plain();
        let mut reopened = Panel::new(&request, Some(&panel.draft(&request)));
        assert_eq!(steps(&request, &reopened, width).plain(), shown);
        assert!(reopened.switch(&request, true));
        let next = if wide {
            "  ◉ Ambito della pulizia   Formati   ◉ Consegna"
        } else {
            back
        };
        starts(&request, &reopened, width, next, 1 + usize::from(wide));
    }
}

#[test]
fn long_titles_clip_without_crowding_or_orphaning_a_mark() {
    let titles = [
        "Ambito della pulizia dei dati storici",
        "Formati",
        "Consegna dei risultati al team",
        "Note",
    ];
    let questions = titles
        .iter()
        .enumerate()
        .map(|(id, title)| {
            format!(
                r#"{{"id":"q{id}","title":"{title}","question":"Pick","options":[{{"id":"a","label":"A"}}]}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let request = request(&format!(r#"{{"questions":[{questions}]}}"#));
    // Every width from narrow to wide, every current step and every set of
    // answered questions; `steps` checks each row's marks and width.
    for width in 24..=80 {
        for step in 0..titles.len() {
            for answered in 0..1u8 << titles.len() {
                let mut panel = Panel::new(&request, None);
                panel.step = step;
                for (index, entry) in panel.entries.iter_mut().enumerate() {
                    if answered & 1 << index != 0 {
                        entry.picked = vec![0];
                    }
                }
                let row = steps(&request, &panel, width);
                let count = format!("{} of {}", step + 1, titles.len());
                let later = usize::from(step + 1 < titles.len());
                if text::cells(titles[step]) + text::cells(&count) + 6 + later <= width {
                    assert!(
                        row.plain().contains(titles[step]),
                        "{width}: {:?}",
                        row.plain()
                    );
                }
            }
        }
    }
}

#[test]
fn the_current_title_takes_priority_over_elided_earlier_titles() {
    let title = "Delivery instructions for team";
    for step in [1, 2] {
        let mut request = request(CLEANUP);
        request.questions[step].title = title.into();
        let mut panel = Panel::new(&request, None);
        panel.step = step;
        // Indent, count, count gap and right margin; reserve an ellipsis
        // when later titles remain. Earlier titles must not steal this room.
        let minimum = text::cells(title) + 12 + usize::from(step == 1);
        for width in minimum..minimum + 4 {
            let row = steps(&request, &panel, width);
            assert!(row.plain().contains(title), "{width}: {:?}", row.plain());
        }
    }
}

#[test]
fn the_mark_keeps_its_title_where_a_cut_would_separate_them() {
    let request = request(CLEANUP);
    let mut panel = Panel::new(&request, None);
    panel.entries[1].picked = vec![0];
    panel.entries[2].editor.replace("Via email".into());
    panel.entries[2].other = true;
    // `  Ambito della pulizia   ◉ Formati` leaves the mark at the cut for
    // some widths; the title goes with it rather than leaving `◉…`.
    for width in 30..=60 {
        let plain = steps(&request, &panel, width).plain();
        assert!(
            !plain.contains("◉…") && !plain.contains("◉ …"),
            "{width}: {plain:?}"
        );
    }
}
