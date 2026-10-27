mod commands;
mod composer;
mod footer;
mod information;
mod menus;
pub(super) mod opening;
mod panel;
pub(super) mod processes;
mod quit;
mod routing;
mod status;
#[cfg(test)]
use super::text;
use super::{line::Line, state::State};

pub struct Frame {
    #[cfg(test)]
    pub header: Vec<Line>,
    #[cfg(test)]
    pub history: Vec<Vec<Line>>,
    pub live: Vec<Line>,
    pub cursor: Option<(usize, usize)>,
    pub composer: usize,
}

pub(super) struct Chrome {
    lines: Vec<Line>,
    cursor: Option<(usize, usize)>,
}

impl Chrome {
    pub fn capacity(&self, height: usize) -> usize {
        let available = height.saturating_sub(self.lines.len());
        available.saturating_sub(usize::from(available >= 2))
    }
}

pub(super) fn chrome(state: &State, model: &str, directory: &str, reading: bool) -> Chrome {
    let (lines, cursor) = composer::render(
        state,
        model,
        directory,
        state.width.max(1),
        chrome_height(state).min(state.height),
        reading,
    );
    Chrome { lines, cursor }
}

#[cfg(test)]
pub fn frame(state: &State, model: &str, directory: &str) -> Frame {
    let mut history = state.rows();
    let end = state.settled_len();
    let pending = history.split_off(end);
    let mut frame = lower(
        state,
        model,
        directory,
        opening::rows(state, state.width.max(2))
            .into_iter()
            .chain(history.iter().flatten().cloned())
            .chain(pending.into_iter().flatten())
            .collect(),
        state.height,
        false,
    );
    frame.header = opening::rows(state, state.width.max(2));
    frame.history = history;
    frame
}

#[cfg(test)]
pub fn composer_frame(state: &State, model: &str, directory: &str) -> Frame {
    let (live, cursor) = composer::render(
        state,
        model,
        directory,
        state.width.max(1),
        chrome_height(state),
        false,
    );
    Frame {
        header: vec![],
        history: vec![],
        live,
        cursor,
        composer: 0,
    }
}

#[cfg(test)]
pub(super) fn lower(
    state: &State,
    model: &str,
    directory: &str,
    live: Vec<Line>,
    height: usize,
    back_to_bottom: bool,
) -> Frame {
    let chrome = chrome(state, model, directory, back_to_bottom);
    lower_with_chrome(live, height, chrome)
}

pub(super) fn lower_with_chrome(mut live: Vec<Line>, height: usize, mut chrome: Chrome) -> Frame {
    let available = height.saturating_sub(chrome.lines.len());
    // Keep room for a content row as well as the gap. The gap belongs to the
    // temporary input area, so it never becomes saved history on its own.
    let separated = available >= 2;
    let capacity = available.saturating_sub(usize::from(separated));
    if live.len() > capacity {
        live.drain(..live.len() - capacity);
    }
    live.resize(capacity, Line::default());
    if separated {
        live.push(Line::default());
    }
    let composer = live.len();
    let cursor = chrome.cursor.map(|(row, column)| (composer + row, column));
    live.append(&mut chrome.lines);
    Frame {
        #[cfg(test)]
        header: vec![],
        #[cfg(test)]
        history: vec![],
        live,
        cursor,
        composer,
    }
}

pub(super) fn panel_open(state: &State) -> bool {
    state.quitting.is_some()
        || state.processes.is_some()
        || state.selector.is_some()
        || state.information.is_some()
        || state.suggestions.panel
}

pub(super) fn chrome_height(state: &State) -> usize {
    if state.height >= 12 && panel_open(state) {
        state.height * 2 / 3
    } else {
        state.height
    }
}

#[cfg(test)]
pub(super) fn capacity(state: &State, model: &str, directory: &str) -> usize {
    chrome(state, model, directory, false).capacity(state.height)
}

/// The draft's wrapping width in the composer.
pub(super) fn input_width(width: usize) -> usize {
    width.saturating_sub(3).max(1)
}

/// Log text width of the open process stream.
pub(super) fn process_width(state: &State) -> usize {
    processes::text_width(state.width.max(1))
}

/// Process list entries visible at once.
pub(super) fn process_capacity(state: &State) -> usize {
    composer::panel_room(state)
}

/// Total and visible rows of the open process stream.
pub(super) fn process_log_rows(state: &State) -> (usize, usize) {
    state.processes.as_ref().map_or((0, 0), |panel| {
        processes::metrics(panel, state.width.max(1), composer::panel_room(state))
    })
}

/// The gap row while reading back: `↓` at column 0, in ACCENT once output
/// arrived after scrolling away, the rows below, and the way back as a
/// keycap ending two cells from the edge; the keycap drops first.
pub(super) fn reading_row(columns: usize, below: usize, fresh: bool, alt_end: bool) -> Line {
    use super::theme::{ACCENT, DIM, LIT, MUTED};
    let mut line = Line::new("↓ ", if fresh { ACCENT } else { DIM }.bold());
    line.push(
        &format!("{below} row{} below", if below == 1 { "" } else { "s" }),
        DIM,
    );
    if fresh {
        line.push(" · ", DIM);
        line.push("new output", LIT);
    }
    let mut key = panel::keycap(if alt_end { "Alt+End" } else { "Esc" });
    key.push(" back to bottom", MUTED);
    let end = columns.saturating_sub(2);
    let (left, right) = (
        super::text::cells(&line.plain()),
        super::text::cells(&key.plain()),
    );
    if left + 2 + right > end {
        return line.shortened(end.max(1));
    }
    line.push(&" ".repeat(end - left - right), DIM);
    line.append(key)
}

pub(super) fn scroll_information(state: &mut State, code: u16) {
    let room = composer::panel_room(state);
    if let Some(info) = &mut state.information {
        information::scroll(info, code, state.width.max(1), room);
    }
}

/// Whether `line` is a panel's focused row.
#[cfg(test)]
pub(super) fn focused(line: &Line) -> bool {
    line.spans
        .iter()
        .any(|span| span.background == Some(super::theme::SELECTED_SURFACE))
}

#[cfg(test)]
mod composer_tests;
#[cfg(test)]
mod delete_tests;
#[cfg(test)]
mod footer_tests;
#[cfg(test)]
mod information_tests;
#[cfg(test)]
mod navigation_tests;
#[cfg(test)]
mod panel_tests;
#[cfg(test)]
mod resize_tests;
#[cfg(test)]
mod startup_tests;
#[cfg(test)]
mod tests;
