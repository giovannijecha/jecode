use super::panel::{self, key};
use crate::tui::{
    line::Line,
    motion,
    state::{Item, State},
    text,
    theme::{ACCENT, BODY, DIM, GOOD, LIT, MUTED, STRONG},
};

const MARK_WIDTH: usize = 49;
/// With the keys, without them, with the compact mark.
const LEVELS: usize = 3;
/// Setup advice while no model is selected; otherwise the saved conversations.
const START: [(&str, &str); 3] = [
    ("/model", "Choose the model"),
    ("/permissions", "Set what Jecode may change here"),
    ("/help", "Show commands and keys"),
];

/// While the conversation is empty, the mark and one block inside its two edges. A first
/// prompt restores the one-line header and the ordinary transcript grid.
pub(in crate::tui) fn rows(state: &State, columns: usize) -> Vec<Line> {
    if state.items.iter().any(Item::is_visible) {
        return slim(state, columns);
    }
    let room = room(state, columns);
    if columns >= MARK_WIDTH + 4 {
        for level in 0..LEVELS {
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
    (0..LEVELS).any(|level| splash(state, state.width, level).len() <= room)
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
    let mut title = Line::new(">_", ACCENT.bold());
    title.push(" jecode", STRONG.bold());
    title.push(&format!(" {}", env!("CARGO_PKG_VERSION")), MUTED);
    let mut rows = vec![title.shortened(columns), Line::default()];
    if state.items.iter().any(Item::is_visible) {
        return rows;
    }
    let width = panel::text_width(columns);
    rows.extend(
        content(state, width)
            .into_iter()
            .map(|line| panel::indent(line, columns)),
    );
    rows
}

/// Everything inside the mark's two edges: version and facts, then what comes next, then
/// the keys. Short or narrow, the keys leave, then the mark turns compact.
fn splash(state: &State, columns: usize, level: usize) -> Vec<Line> {
    let mut rows = wordmark(level == 2);
    rows.push(Line::default());
    rows.push(facts(state));
    rows.push(Line::default());
    rows.extend(content(state, MARK_WIDTH));
    if level == 0 {
        rows.push(Line::default());
        rows.push(keys(MARK_WIDTH));
    }
    let indent = " ".repeat(columns.saturating_sub(MARK_WIDTH) / 2);
    rows.into_iter()
        .map(|line| line.indent(&indent, MUTED))
        .collect()
}

fn facts(state: &State) -> Line {
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
    panel::spread(
        Line::new(env!("CARGO_PKG_VERSION"), MUTED),
        facts,
        MARK_WIDTH,
        None,
    )
}

/// A selected model without conversations gets an empty state, not setup advice.
fn content(state: &State, width: usize) -> Vec<Line> {
    if state.opening.recent.is_empty() {
        if !state.opening.needs_model {
            return vec![
                Line::new("No saved conversations in this folder", MUTED).shortened(width),
            ];
        }
        let names = START.iter().map(|(name, _)| name.len()).max().unwrap_or(0) + 2;
        let mut rows = vec![Line::new("Start here", DIM)];
        rows.extend(START.iter().map(|(name, fact)| {
            let line = panel::fit(Line::new(name, STRONG), names, None);
            line.append(Line::new(fact, MUTED)).shortened(width)
        }));
        return rows;
    }
    let mut resume = panel::keycap("/resume");
    resume.push(" all", DIM);
    let mut rows = vec![panel::spread(
        Line::new("Recent in this folder", DIM),
        resume,
        width,
        None,
    )];
    for recent in state.opening.recent.iter().take(3) {
        rows.push(panel::spread(
            Line::new(&text::clean(&recent.title), BODY),
            Line::new(&recent.age, MUTED),
            width,
            None,
        ));
    }
    rows
}

/// Nothing to expand yet, so Ctrl+O is absent.
fn keys(width: usize) -> Line {
    panel::row(
        &[
            key("/", "commands"),
            // Option types a character on a Mac unless the terminal maps it to Alt.
            key(
                if cfg!(target_os = "macos") {
                    "Ctrl+V"
                } else {
                    "Alt+V"
                },
                "image",
            ),
            key("Ctrl+Q", "quit"),
        ],
        width,
    )
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
