mod commands;
mod composer;
mod footer;
mod information;
mod menus;
mod notices;
mod panel;
pub(super) mod processes;
mod routing;
use super::{line::Line, state::State, text, theme::MUTED};

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
        super::transcript::header(state.width.max(2))
            .into_iter()
            .chain(history.iter().flatten().cloned())
            .chain(pending.into_iter().flatten())
            .collect(),
        state.height,
        false,
    );
    frame.header = super::transcript::header(state.width.max(2));
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
    state.processes.is_some()
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

pub(super) fn fitted(left: &str, right: &str, columns: usize) -> Line {
    let right = text::ellipsize(&text::clean(right), columns.saturating_sub(2));
    let budget = columns.saturating_sub(text::cells(&right) + 2);
    let left = text::ellipsize(&text::clean(left), budget);
    let gap = columns.saturating_sub(text::cells(&left) + text::cells(&right));
    Line::new(&format!("{left}{}{right}", " ".repeat(gap)), MUTED)
}

pub(super) fn input_width(width: usize, command_panel: bool) -> usize {
    let columns = width;
    let columns = if command_panel {
        panel::width(columns)
    } else {
        columns
    };
    columns.saturating_sub(3).max(1)
}

/// Log text width of the open process stream.
pub(super) fn process_width(state: &State) -> usize {
    processes::text_width(panel::width(state.width.max(1)))
}

/// Process list entries visible at once.
pub(super) fn process_capacity(state: &State) -> usize {
    processes::list_capacity(composer::information_height(state))
}

/// Total and visible rows of the open process stream.
pub(super) fn process_log_rows(state: &State) -> (usize, usize) {
    state.processes.as_ref().map_or((0, 0), |panel| {
        processes::metrics(
            panel,
            panel::width(state.width.max(1)),
            composer::information_height(state),
        )
    })
}

pub(super) fn scroll_information(state: &mut State, code: u16) {
    let height = composer::information_height(state);
    if let Some(info) = &mut state.information {
        information::scroll(info, code, state.width.max(1), height);
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
mod tests;
