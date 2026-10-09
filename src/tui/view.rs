mod composer;
mod information;
mod menus;
mod panel;
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

#[cfg(test)]
pub fn frame(state: &State, model: &str, directory: &str) -> Frame {
    let mut history = state.rows();
    let end = state.settled_len();
    let pending = history.split_off(end);
    let has_content = history.iter().chain(&pending).any(|rows| !rows.is_empty());
    let mut frame = lower(
        state,
        model,
        directory,
        super::transcript::header(state.width.max(2))
            .into_iter()
            .chain(history.iter().flatten().cloned())
            .chain(pending.into_iter().flatten())
            .collect(),
        has_content,
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

pub(super) fn lower(
    state: &State,
    model: &str,
    directory: &str,
    mut live: Vec<Line>,
    has_content: bool,
    height: usize,
    back_to_bottom: bool,
) -> Frame {
    let columns = state.width.max(1);
    let (mut chrome, cursor) = composer::render(
        state,
        model,
        directory,
        columns,
        chrome_height(state).min(height),
        back_to_bottom,
    );
    let available = height.saturating_sub(chrome.len());
    // Keep room for a content row as well as the gap. The gap belongs to the
    // temporary input area, so it never becomes saved history on its own.
    let separated = has_content && available >= 2;
    let capacity = available.saturating_sub(usize::from(separated));
    if live.len() > capacity {
        live.drain(..live.len() - capacity);
    }
    live.resize(capacity, Line::default());
    if separated {
        live.push(Line::default());
    }
    let composer = live.len();
    let cursor = cursor.map(|(row, column)| (composer + row, column));
    live.append(&mut chrome);
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

pub(super) fn chrome_height(state: &State) -> usize {
    if state.height >= 12
        && (state.selector.is_some() || state.information.is_some() || state.suggestions.panel)
    {
        state.height * 2 / 3
    } else {
        state.height
    }
}

pub(super) fn capacity(state: &State, model: &str, directory: &str) -> usize {
    let (chrome, _) = composer::render(
        state,
        model,
        directory,
        state.width.max(1),
        chrome_height(state),
        false,
    );
    let available = state.height.saturating_sub(chrome.len());
    available.saturating_sub(usize::from(available >= 2))
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

pub(super) fn scroll_information(state: &mut State, code: u16) {
    let height = composer::information_height(state);
    if let Some(info) = &mut state.information {
        information::scroll(info, code, state.width.max(1), height);
    }
}

#[cfg(test)]
mod composer_tests;
#[cfg(test)]
mod delete_tests;
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
