use super::panel::{self, key};
use crate::tui::{
    line::Line,
    motion,
    state::{Item, State},
    text,
    theme::{ACCENT, BODY, DIM, GOOD, LIT, MUTED, STRONG},
};

const MARK_WIDTH: usize = 49;

/// The B13 startup splash occupies the empty conversation. A first prompt
/// restores the one-line header and the ordinary transcript grid.
pub(in crate::tui) fn rows(state: &State, columns: usize) -> Vec<Line> {
    if state.items.iter().any(Item::is_visible) {
        return slim(state, columns);
    }
    let room = room(state, columns);
    if columns >= MARK_WIDTH + 4 {
        for level in 0..4 {
            let body = splash(state, columns, level);
            if body.len() <= room {
                let top = (room - body.len()) / 2;
                return std::iter::repeat_with(Line::default)
                    .take(top)
                    .chain(body)
                    .collect();
            }
        }
    }
    slim(state, columns)
}

/// The large and compact marks share the existing caret's motion clock.
pub(in crate::tui) fn animated(state: &State) -> bool {
    if state.items.iter().any(Item::is_visible) || state.width < MARK_WIDTH + 4 {
        return false;
    }
    let room = room(state, state.width);
    (0..4).any(|level| splash(state, state.width, level).len() <= room)
}

fn room(state: &State, columns: usize) -> usize {
    let dock = super::composer::render(
        state,
        "",
        "",
        columns,
        super::chrome_height(state).min(state.height),
    )
    .0
    .len();
    let available = state.height.saturating_sub(dock);
    available.saturating_sub(usize::from(available >= 2))
}

fn slim(state: &State, columns: usize) -> Vec<Line> {
    let opening = &state.opening;
    let mut title = Line::new(">_", ACCENT.bold());
    title.push(" jecode", STRONG.bold());
    title.push(&format!(" {}", env!("CARGO_PKG_VERSION")), MUTED);
    let mut rows = vec![title.shortened(columns), Line::default()];
    if state.items.iter().any(Item::is_visible) {
        return rows;
    }
    let width = panel::text_width(columns);
    if opening.recent.is_empty() {
        rows.push(panel::indent(keys(width), columns));
        return rows;
    }
    rows.push(panel::indent(
        Line::new("Recent in this folder", DIM),
        columns,
    ));
    for recent in &opening.recent {
        let title = Line::new(&text::clean(&recent.title), BODY);
        let age = Line::new(&recent.age, MUTED);
        rows.push(panel::indent(
            panel::spread(title, age, width, None),
            columns,
        ));
    }
    rows.push(Line::default());
    let resume = [key("/resume", "all conversations")];
    rows.push(panel::indent(panel::row(&resume, width), columns));
    rows
}

fn splash(state: &State, columns: usize, level: usize) -> Vec<Line> {
    let compact = level >= 3;
    let mut rows: Vec<_> = wordmark(compact)
        .into_iter()
        .map(|line| centre(line, columns))
        .collect();
    rows.push(Line::default());
    rows.push(centre(Line::new(env!("CARGO_PKG_VERSION"), MUTED), columns));
    rows.push(Line::default());
    if level == 0 {
        let mut tip = Line::new("Tip ", ACCENT.bold());
        tip.push(" /permissions", STRONG);
        tip.push(" sets what Jecode may change here", MUTED);
        rows.push(centre(tip, columns));
    }
    let mut facts = Line::new("✓ ", GOOD);
    facts.push(
        &format!(
            "{} skill{}",
            state.opening.skills,
            if state.opening.skills == 1 { "" } else { "s" }
        ),
        MUTED,
    );
    facts.push("   ", MUTED);
    if state.opening.agents {
        facts.push("✓ ", GOOD);
        facts.push("AGENTS.md", MUTED);
    } else {
        facts.push("– no AGENTS.md", DIM);
    }
    rows.push(centre(facts, columns));
    if level < 2 || state.opening.recent.is_empty() {
        rows.push(Line::default());
        rows.push(centre(keys(columns.saturating_sub(4)), columns));
    }
    if !state.opening.recent.is_empty() {
        rows.push(Line::default());
        rows.extend(recents(state).into_iter().map(|line| centre(line, columns)));
    }
    rows
}

fn keys(width: usize) -> Line {
    panel::row(
        &[
            key("/", "commands"),
            key("Alt+V", "image"),
            key("Ctrl+O", "expand"),
            key("Ctrl+Q", "quit"),
        ],
        width,
    )
}

fn recents(state: &State) -> Vec<Line> {
    let mut rows = vec![panel::fit(
        Line::new("Recent in this folder", DIM),
        MARK_WIDTH,
        None,
    )];
    for recent in state.opening.recent.iter().take(3) {
        rows.push(panel::spread(
            Line::new(&text::clean(&recent.title), BODY),
            Line::new(&recent.age, MUTED),
            MARK_WIDTH,
            None,
        ));
    }
    rows.push(Line::default());
    rows.push(panel::fit(
        panel::row(&[key("/resume", "all conversations")], MARK_WIDTH),
        MARK_WIDTH,
        None,
    ));
    rows
}

fn centre(line: Line, columns: usize) -> Line {
    let indent = columns.saturating_sub(text::cells(&line.plain())) / 2;
    line.indent(&" ".repeat(indent), MUTED)
}

// Five-cell glyphs with two-cell strokes. One cell separates letters, and
// three cells separate >_ from jecode: 11 + 3 + 35 = 49 cells.
fn glyph(character: char) -> [&'static str; 5] {
    match character {
        '>' => ["##...", ".##..", "..##.", ".##..", "##..."],
        '_' => [".....", ".....", ".....", ".....", "#####"],
        'j' => ["...##", "...##", "...##", "##.##", ".###."],
        'e' => ["#####", "##...", "####.", "##...", "#####"],
        'c' => ["#####", "##...", "##...", "##...", "#####"],
        'o' => ["#####", "##.##", "##.##", "##.##", "#####"],
        'd' => ["####.", "##.##", "##.##", "##.##", "####."],
        _ => unreachable!("wordmark characters are fixed"),
    }
}

fn wordmark(compact: bool) -> Vec<Line> {
    let mut pixels: [_; 5] = std::array::from_fn(|_| Vec::with_capacity(MARK_WIDTH));
    for (part_index, part) in [">_", "jecode"].into_iter().enumerate() {
        if part_index > 0 {
            for row in &mut pixels {
                row.extend([None; 3]);
            }
        }
        for (index, character) in part.chars().enumerate() {
            if index > 0 {
                for row in &mut pixels {
                    row.push(None);
                }
            }
            let pattern = glyph(character);
            for (row, bits) in pixels.iter_mut().zip(pattern) {
                row.extend(bits.bytes().map(|bit| {
                    (bit == b'#').then_some(if character == '_' { LIT } else { ACCENT })
                }));
            }
        }
    }
    let caret_on = (motion::frame() / 4).is_multiple_of(2);
    let visible = |style: Option<_>| style.filter(|style| *style != LIT || caret_on);
    if !compact {
        return pixels
            .into_iter()
            .map(|row| {
                let mut line = Line::default();
                for style in row.into_iter().map(visible) {
                    line.push(
                        if style.is_some() { "▀" } else { " " },
                        style.unwrap_or(MUTED),
                    );
                }
                line
            })
            .collect();
    }
    (0..5)
        .step_by(2)
        .map(|y| {
            let mut line = Line::default();
            for x in 0..MARK_WIDTH {
                let top = visible(pixels[y][x]);
                let bottom = pixels.get(y + 1).and_then(|row| visible(row[x]));
                let block = match (top, bottom) {
                    (Some(_), Some(_)) => "█",
                    (Some(_), None) => "▀",
                    (None, Some(_)) => "▄",
                    (None, None) => " ",
                };
                line.push(block, top.or(bottom).unwrap_or(MUTED));
            }
            line
        })
        .collect()
}
