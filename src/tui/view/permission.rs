//! A permission request takes the composer's place (B12.4): below the
//! `? Waiting for permission` pill, between two `accent` rules, the call as
//! it will run, its access and three choices, then the keys. The title and
//! the level chip (B12.8) ride the top rule.

use super::{
    composer, footer,
    panel::{self, Key, Part, key},
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
    let mut parts = vec![
        (status, Part::Chrome),
        (rule(&title(pending), &cards::level(level)), Part::Keep),
    ];
    for row in facts(pending, panel::text_width(columns)) {
        parts.push((panel::indent(row, columns), Part::Keep));
    }
    parts.push((Line::default(), Part::Space));
    for (row, on) in choices(pending, columns) {
        parts.push((row, if on { Part::Focus } else { Part::Keep }));
    }
    parts.push((rule(&[], &Line::default()), Part::Chrome));
    if columns >= 7 {
        for row in panel::keys(&keys(pending), columns - 4, false) {
            parts.push((row.indent("  ", MUTED), Part::Keep));
        }
    }
    panel::fitted(parts, None, height)
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

/// The tool name in its family colour and the target in full, wrapping under
/// itself and never cut; then the access.
fn facts(pending: &Pending, width: usize) -> Vec<Line> {
    let access = &pending.request.access;
    let head = cards::tool_title(&pending.request.tool).append(Line::new(" ", MUTED));
    let indent = panel::cells(&head);
    let target = match access.kind {
        Kind::Runs => text::clean(&access.target),
        Kind::Reads | Kind::Edits => footer::path(&access.target),
    };
    let mut call = Line::new(&target, STRONG);
    if let Some(place) = &access.place {
        call.push(&format!(" in {}", footer::path(place)), MUTED);
    }
    let mut rows: Vec<Line> = call
        .wrap(width.saturating_sub(indent).max(1), true)
        .into_iter()
        .enumerate()
        .map(|(index, part)| {
            if index == 0 {
                head.clone().append(part)
            } else {
                Line::new(&" ".repeat(indent), MUTED).append(part)
            }
        })
        .collect();
    rows.push(Line::new(wording::access(access), MUTED).shortened(width));
    rows
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

/// 1–3 choose while no row has the focus; Enter once ↑↓ placed it.
fn keys(pending: &Pending) -> Vec<Key<'static>> {
    let choose = if pending.focus.is_some() {
        key("Enter", "choose")
    } else {
        key("1–3", "choose")
    };
    vec![choose, key("↑↓", "move").optional()]
}
