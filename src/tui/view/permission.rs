//! A permission request takes the composer's place (B12.4): below the
//! `? Waiting for permission` pill, between two `accent` rules, the call as
//! it will run, its access and three choices, then the keys. The title and
//! the level chip (B12.8) ride the top rule.

use super::{
    composer, footer,
    panel::{self, Part, key},
};
use crate::{
    permissions::{Kind, wording},
    tui::{
        cards,
        line::Line,
        permission::Pending,
        text,
        theme::{ACCENT, MUTED, SELECTED_SURFACE, STRONG},
    },
};

const CHOICES: [&str; 3] = ["Approve", "Approve and remember", "Deny"];

pub(super) fn block(
    pending: &Pending,
    level: crate::permissions::Level,
    status: Line,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let rule = |label: &[Line], aside: &Line| composer::rule(columns, ACCENT, label, aside);
    let layout = Layout::new(pending, columns, height);
    let mut parts = vec![
        (status, Part::Chrome),
        (rule(&title(pending), &cards::level(level)), Part::Keep),
    ];
    for row in layout.facts(pending, panel::text_width(columns)) {
        parts.push((panel::indent(row, columns), Part::Keep));
    }
    parts.push((Line::default(), Part::Space));
    for (row, on) in layout.choices {
        parts.push((row, if on { Part::Focus } else { Part::Keep }));
    }
    parts.push((rule(&[], &Line::default()), Part::Chrome));
    for row in layout.keys {
        parts.push((row.indent("  ", MUTED), Part::Keep));
    }
    panel::fitted(parts, None, height)
}

/// Where PgUp or PgDn leaves a call too long for the dock; `None` when all of
/// it shows, and those keys scroll the conversation as ever.
pub(super) fn page(pending: &Pending, columns: usize, height: usize, back: bool) -> Option<usize> {
    let layout = Layout::new(pending, columns, height);
    let last = layout.call.len().checked_sub(layout.room)?;
    if last == 0 {
        return None;
    }
    // One row stays in view, so a line is never lost across the page.
    let step = layout.room.saturating_sub(1).max(1);
    Some(if back {
        layout.start.saturating_sub(step)
    } else {
        (layout.start + step).min(last)
    })
}

/// The rows a request needs with one row of its call: its choices and keys
/// outrank the panels' two-thirds cap (B12.4).
pub(super) fn floor(pending: &Pending, columns: usize) -> usize {
    let layout = Layout::new(pending, columns, 0);
    3 + layout.choices.len() + layout.keys.len()
}

/// How the call shares the dock. It is what the user agrees to, so it is
/// never cut, and the choices and keys never leave: what does not fit scrolls.
struct Layout {
    choices: Vec<(Line, bool)>,
    keys: Vec<Line>,
    /// The tool name, which stays while the call scrolls.
    head: Line,
    call: Vec<Line>,
    room: usize,
    start: usize,
}

impl Layout {
    fn new(pending: &Pending, columns: usize, height: usize) -> Self {
        let choices = choices(pending, columns);
        let head = cards::tool_title(&pending.request.tool).append(Line::new(" ", MUTED));
        let width = panel::text_width(columns).saturating_sub(panel::cells(&head));
        let call = call(pending, width.max(1));
        // The top rule and the access row always stay beside the choices and keys.
        let room = |keys: &[Line]| height.saturating_sub(2 + choices.len() + keys.len()).max(1);
        let mut keys = key_rows(pending, None, columns);
        let mut shown = room(&keys);
        let mut start = 0;
        if call.len() > shown {
            // Measured at its widest, as the key may take a row of its own.
            shown = room(&key_rows(pending, Some(call.len()), columns));
            start = pending.scroll.min(call.len() - shown);
            keys = key_rows(pending, Some(call.len() - shown), columns);
        }
        Self {
            choices,
            keys,
            head,
            call,
            room: shown,
            start,
        }
    }

    /// The rows of the call in view, under the tool name, then the access.
    fn facts(&self, pending: &Pending, width: usize) -> Vec<Line> {
        let indent = Line::new(&" ".repeat(panel::cells(&self.head)), MUTED);
        let mut rows: Vec<Line> = self
            .call
            .iter()
            .skip(self.start)
            .take(self.room)
            .enumerate()
            .map(|(index, part)| {
                let lead = if index == 0 { &self.head } else { &indent };
                lead.clone().append(part.clone())
            })
            .collect();
        rows.push(Line::new(wording::access(&pending.request.access), MUTED).shortened(width));
        rows
    }
}

/// `Permission  MAIN AGENT  · 1 of 3`, then without the count.
fn title(pending: &Pending) -> Vec<Line> {
    let head = Line::new("Permission", STRONG.bold())
        .append(Line::new(" ", MUTED))
        .append(cards::agent_badge(&pending.agent, "done"));
    let mut counted = head.clone();
    if pending.waiting > 1 {
        counted.push(&format!(" · 1 of {}", pending.waiting), MUTED);
    }
    vec![counted, head]
}

/// The target, or every file of a multi-file edit, wrapping under itself.
fn call(pending: &Pending, width: usize) -> Vec<Line> {
    let access = &pending.request.access;
    let call = if access.members.is_empty() {
        let target = match access.kind {
            Kind::Runs => text::clean(&access.target),
            Kind::Reads | Kind::Edits => footer::path(&access.target),
        };
        let mut call = Line::new(&target, STRONG);
        if let Some(place) = &access.place {
            call.push(&format!(" in {}", footer::path(place)), MUTED);
        }
        call
    } else {
        let paths: Vec<_> = access
            .members
            .iter()
            .map(|file| footer::path(&file.target))
            .collect();
        Line::new(&paths.join(", "), STRONG)
    };
    call.wrap(width, true)
}

/// The choices as option rows. A fact is what the user agrees to, so it is
/// never cut: when one does not fit, every fact wraps under its name.
fn choices(pending: &Pending, columns: usize) -> Vec<(Line, bool)> {
    let access = &pending.request.access;
    let facts = [
        wording::once(access).to_owned(),
        format!(
            "{}, this conversation",
            wording::covers(access, &pending.request.tool)
        ),
        wording::DENY.to_owned(),
    ];
    let end = columns.saturating_sub(2);
    let column = 2
        + CHOICES
            .iter()
            .map(|name| text::cells(name))
            .max()
            .unwrap_or(0)
        + 2;
    let inline = facts.iter().all(|fact| column + text::cells(fact) <= end);
    let mut rows = Vec::new();
    for (index, (name, fact)) in CHOICES.iter().zip(&facts).enumerate() {
        let on = pending.focus == Some(index);
        let mut head = panel::marker(on, false);
        head.push(name, STRONG);
        let lines = if inline {
            let head = panel::fit(head, column, None);
            vec![head.append(Line::new(fact, MUTED))]
        } else {
            let mut lines = vec![head];
            for part in Line::new(fact, MUTED).wrap(end.saturating_sub(4).max(1), true) {
                lines.push(Line::new("    ", MUTED).append(part));
            }
            lines
        };
        for line in lines {
            let fill = on.then_some(SELECTED_SURFACE);
            let mut line = panel::fit(line, columns, fill);
            if let Some(fill) = fill {
                line = panel::surfaced(line, fill);
            }
            rows.push((line, on));
        }
    }
    rows
}

/// 1–3 choose while no row has the focus; Enter once ↑↓ placed it. `hidden`
/// counts the rows of the call that PgUp/PgDn can bring into view.
fn key_rows(pending: &Pending, hidden: Option<usize>, columns: usize) -> Vec<Line> {
    if columns < 7 {
        return Vec::new();
    }
    let hidden = hidden.map(|rows| format!("+{rows} {}", if rows == 1 { "line" } else { "lines" }));
    let mut keys = vec![if pending.focus.is_some() {
        key("Enter", "choose")
    } else {
        key("1–3", "choose")
    }];
    if let Some(hidden) = &hidden {
        keys.push(key("PgUp/PgDn", hidden));
    }
    keys.push(key("↑↓", "move").optional());
    panel::keys(&keys, columns - 4, false)
}
