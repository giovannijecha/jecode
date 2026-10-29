//! `/agents`: the B5 list of this conversation's collaborators, and inside
//! one, its own conversation in the main renderer with a single row of keys.

use super::{
    Chrome, Frame,
    panel::{self, Field, Part, Sheet, key},
};
use crate::{
    json::Value,
    tui::{
        agents::{self, Panel, conversation},
        cards,
        line::Line,
        state::{State, rows::Rows},
        text,
        theme::{BAD, MUTED, STRONG, Style, WARNING},
    },
};
use std::{cell::RefCell, sync::Arc};

/// `4m`, `1h 12m`: how long a collaborator has been running.
fn age(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        _ => format!("{}h {}m", seconds / 3600, seconds / 60 % 60),
    }
}

/// The state in the fact column: failed checks `bad`, unknown outcomes
/// `warning`, everything routine muted.
fn fact(state: &State, record: &Value) -> (String, Style) {
    let status = agents::status(record);
    let style = match agents::value(record, "state") {
        "unknown" => WARNING,
        "failed" if status != "reviewed" => BAD,
        "returned" if agents::failed_check(record).is_some() && status != "reviewed" => BAD,
        _ => MUTED,
    };
    let fact = match state.agent_clocks.get(agents::value(record, "id")) {
        Some(started) if status == "running" => format!("running · {}", age(started.elapsed())),
        _ => status.into(),
    };
    (fact, style)
}

fn name(record: &Value) -> Line {
    let state = match agents::value(record, "state") {
        "returned" if agents::failed_check(record).is_some() => "failed",
        state => state,
    };
    let mut line = cards::agent_badge(&agents::label(record), state);
    line.push(
        &format!(" {}", text::clean(agents::value(record, "title"))),
        STRONG,
    );
    line
}

fn list_rows(state: &State, panel_state: &Panel, columns: usize, room: usize) -> Vec<(Line, Part)> {
    let filtered = panel_state.filtered(&state.collaboration);
    if filtered.is_empty() {
        return vec![(
            panel::indent(
                Line::new(
                    if state.collaboration.is_empty() {
                        "No open agents"
                    } else {
                        "No matching agents"
                    },
                    MUTED,
                ),
                columns,
            ),
            Part::Keep,
        )];
    }
    let end = columns.saturating_sub(2);
    // The fact column: the widest name plus two, at most 3/5 of the text.
    let column = filtered
        .iter()
        .map(|&index| panel::cells(&name(&state.collaboration[index])) + 2)
        .max()
        .unwrap_or(0)
        .min(panel::text_width(columns) * 3 / 5);
    let start = panel::start(panel_state.selected, filtered.len(), room);
    let shown = filtered
        .iter()
        .enumerate()
        .skip(start)
        .take(room)
        .map(|(index, record_index)| {
            let record = &state.collaboration[*record_index];
            let on = index == panel_state.selected;
            let mut line = panel::marker(on, false).append(name(record));
            let (fact, style) = fact(state, record);
            if 2 + column + 8 <= end {
                let at = (2 + column).max(panel::cells(&line) + 2);
                line = panel::fit(line, at, None);
                line.push(&text::ellipsize(&fact, end.saturating_sub(at)), style);
            }
            panel::choice(line, on, columns)
        })
        .collect();
    panel::window(shown, start, filtered.len(), room, columns)
        .into_iter()
        .enumerate()
        .map(|(offset, line)| {
            (
                line,
                if start + offset == panel_state.selected {
                    Part::Keep
                } else {
                    Part::Row((start + offset).abs_diff(panel_state.selected))
                },
            )
        })
        .collect()
}

pub(super) fn sheet<'a>(state: &'a State, panel_state: &'a Panel, columns: usize) -> Sheet<'a> {
    let width = panel::text_width(columns);
    let filtered = panel_state.filtered(&state.collaboration);
    let room = columns.saturating_sub(3).max(1);
    let (value, caret) = panel_state.filter.viewport(room, false);
    Sheet {
        title: panel::title(
            None,
            "Agents",
            &format!("{} open", state.collaboration.len()),
        ),
        aside: Line::default(),
        field: Some(Field {
            label: "Filter agents".into(),
            value,
            caret,
            placeholder: "",
            suffix: "",
        }),
        draft_caret: false,
        head: Vec::new(),
        body: Box::new(move |room| list_rows(state, panel_state, columns, room)),
        total: Some(filtered.len()),
        tail: Box::new(move |overflow| {
            panel::keys(
                &[
                    key("Enter", "open"),
                    key("↑↓", "move").scrolling(),
                    key("Esc", "close"),
                ],
                width,
                overflow,
            )
        }),
        confirming: false,
    }
}

/// The open collaborator's rendered conversation, kept until its record,
/// changes or width change. Only the live activity line, which the rows do
/// not show, changes without a redraw.
#[derive(Default)]
pub(in crate::tui) struct Cache(RefCell<Option<Cached>>);

struct Cached {
    columns: usize,
    record: Value,
    rows: Arc<Vec<Line>>,
}

impl Cache {
    /// Drops the rows, for changes read after they were drawn.
    pub fn clear(&self) {
        self.0.take();
    }

    #[cfg(test)]
    pub fn rows(&self) -> Option<Arc<Vec<Line>>> {
        self.0
            .borrow()
            .as_ref()
            .map(|cached| Arc::clone(&cached.rows))
    }
}

/// Whether two snapshots draw the same conversation.
fn same_rows(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let drawn = |(key, _): &(&String, &Value)| key.as_str() != "activity";
            left.iter().filter(drawn).eq(right.iter().filter(drawn))
        }
        _ => left == right,
    }
}

/// The open collaborator's conversation rows, drawn as the main one is.
fn conversation_rows(panel: &Panel, record: &Value, columns: usize) -> Arc<Vec<Line>> {
    let mut cache = panel.rows.0.borrow_mut();
    if let Some(cached) = cache.as_ref()
        && cached.columns == columns
        && same_rows(&cached.record, record)
    {
        return Arc::clone(&cached.rows);
    }
    let items = conversation::items(record, &panel.changes.calls);
    let mut rows = Rows::default();
    let rows = Arc::new(
        (0..items.len())
            .flat_map(|index| rows.render_at(&items, index, columns, false))
            .collect::<Vec<_>>(),
    );
    *cache = Some(Cached {
        columns,
        record: record.clone(),
        rows: Arc::clone(&rows),
    });
    rows
}

/// Conversation rows above the foot: one row of keys and the gap above it.
fn room(height: usize) -> usize {
    height.saturating_sub(2).max(1)
}

/// Total rows of the open collaborator's conversation and the rows shown.
pub(super) fn inside_rows(state: &State) -> (usize, usize) {
    let Some(panel) = &state.agents_panel else {
        return (0, 0);
    };
    if panel.inside.is_none() {
        return (0, 0);
    }
    panel
        .current(&state.collaboration)
        .map_or((0, 0), |record| {
            (
                conversation_rows(panel, record, state.width.max(2)).len(),
                room(state.height),
            )
        })
}

/// Inside a collaborator, its conversation replaces the main column. It is
/// read only: no composer, status row, footer or title, only the keys.
pub(super) fn inside(state: &State) -> Option<Frame> {
    let panel = state.agents_panel.as_ref()?;
    panel.inside.as_ref()?;
    let record = panel.current(&state.collaboration)?;
    let columns = state.width.max(2);
    let lines = conversation_rows(panel, record, columns);
    let room = room(state.height);
    let end = lines.len().saturating_sub(room);
    let start = if panel.follow {
        end
    } else {
        panel.offset.min(end)
    };
    let shown = lines.iter().skip(start).take(room).cloned().collect();
    let working = matches!(agents::value(record, "state"), "running" | "stopping");
    let mut keys = vec![key("Esc", "back to agents"), key("↑↓", "scroll")];
    if working {
        keys.push(key("End", "follow"));
    }
    let foot = panel::row(&keys, columns.saturating_sub(4)).indent("  ", MUTED);
    Some(super::lower_with_chrome(
        shown,
        state.height,
        Chrome {
            lines: vec![foot],
            cursor: None,
        },
    ))
}
