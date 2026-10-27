use super::panel::{self, key};
use crate::tui::{
    line::Line,
    state::{Item, State},
    text,
    theme::{ACCENT, BODY, DIM, MUTED, STRONG},
};

/// The header: `>_ jecode` and the version; the footer carries the model,
/// effort and folder. Until the first prompt the startup space follows: the
/// recent conversations in this folder with `/resume`, or on the first run
/// the key row.
pub(in crate::tui) fn rows(state: &State, columns: usize) -> Vec<Line> {
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
        let keys = [
            key("/", "commands"),
            key("Alt+V", "image"),
            key("Ctrl+O", "expand"),
            key("Ctrl+Q", "quit"),
        ];
        rows.push(panel::indent(panel::row(&keys, width), columns));
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
