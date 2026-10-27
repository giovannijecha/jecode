//! The status row: one row above the composer for every turn state and
//! notice. A pill ` glyph label ` on its surface, `dim` facts joined by
//! ` · ` and dropped from the right, then an optional keycap ending two cells
//! from the edge, which drops after the facts.
use super::panel;
use crate::{
    json::Value,
    tui::{
        activity,
        feedback::{Ending, Feedback},
        line::Line,
        motion,
        state::{Item, Kind, State},
        text,
        theme::{BAD, BODY, DIM, GOOD, MUTED, PILL_SURFACE, STRONG, WARNING},
    },
};
use std::time::Instant;

/// The narrowest a lone fact is shortened to before it drops.
const FACT_MIN: usize = 6;
/// The narrowest a label is shortened to so its first fact fits.
const LABEL_MIN: usize = 12;
/// The pill around its label: padding, the two-cell glyph and its gap.
const PILL_CHROME: usize = 5;

/// `escape` says whether Esc interrupts the turn from here: not while a
/// panel, a draft edit or reading back holds it.
pub(super) fn row(state: &State, columns: usize, escape: bool) -> Line {
    let frame = motion::frame();
    if let Some(notice) = shown_notice(state) {
        return notice_row(notice, columns);
    }
    let running = state
        .activity
        .as_ref()
        .filter(|activity| !activity.stopping);
    let time = state
        .activity
        .as_ref()
        .map(|activity| activity::duration(activity.started.elapsed()));
    let mut facts = Vec::new();
    if escape && running.is_some() {
        facts.push(Line::new("Esc interrupt", DIM));
    }
    facts.extend(time.as_deref().map(|time| Line::new(time, DIM)));
    let stopping = state
        .activity
        .as_ref()
        .is_some_and(|activity| activity.stopping);
    if let Some(recovery) = state.recovery.as_ref().filter(|_| !stopping) {
        // `Rate limited · attempt 2 · cause` reads `attempt 2 · Rate limited · cause`.
        let (subject, mut rest) = split(&recovery.text);
        if subject != "Reconnecting" {
            rest.insert(rest.len().min(1), Line::new(&subject, DIM));
        }
        facts.extend(rest);
        let label = Line::new(&countdown(recovery.until), WARNING);
        return fitted(
            motion::caret(frame, Some(WARNING)),
            label,
            facts,
            None,
            columns,
        );
    }
    if let Some(activity) = &state.activity {
        let progress = state.current_progress().filter(|_| !activity.stopping);
        let label = match progress {
            Some(progress) => {
                let (subject, rest) = split(&progress.text);
                facts.extend(rest);
                subject
            }
            None if activity.stopping => "Stopping".into(),
            None => verb(state),
        };
        if progress.is_none() && !activity.stopping {
            facts.extend(
                activity::generated(activity.generated).map(|tokens| Line::new(&tokens, DIM)),
            );
        }
        return fitted(
            motion::caret(frame, None),
            motion::shimmer(&label, frame, false),
            facts,
            None,
            columns,
        );
    }
    if let Some(progress) = state.current_progress() {
        let (subject, rest) = split(&progress.text);
        return fitted(
            motion::caret(frame, None),
            motion::shimmer(&subject, frame, false),
            rest,
            None,
            columns,
        );
    }
    if let Some(ending) = &state.ending {
        return notice_row(ending, columns);
    }
    Line::default()
}

/// The result, warning or error on screen: the more severe of the two
/// slots, the copy slot on a tie.
fn shown_notice(state: &State) -> Option<&Feedback> {
    let settled = [&state.notice, &state.copy_notice]
        .map(|slot| slot.as_ref().filter(|notice| !notice.in_progress()));
    match (settled[0], settled[1]) {
        (Some(notice), Some(copy)) if rank(notice) > rank(copy) => Some(notice),
        (notice, copy) => copy.or(notice),
    }
}

fn rank(notice: &Feedback) -> u8 {
    match notice.kind {
        Kind::Error => 2,
        Kind::Warning => 1,
        _ => 0,
    }
}

/// A result `✓`, warning `▲`, error `✕`, or a turn ending.
fn notice_row(notice: &Feedback, columns: usize) -> Line {
    let glyph = match (notice.ending, notice.kind) {
        (Some(Ending::Done), _) => Line::new(">_", GOOD.bold()),
        (Some(Ending::Stopped), _) => Line::new("■ ", WARNING),
        (Some(Ending::Failed), _) | (None, Kind::Error) => Line::new("✕ ", BAD.bold()),
        (None, Kind::Warning) => Line::new("▲ ", WARNING.bold()),
        (None, _) => Line::new("✓ ", GOOD.bold()),
    };
    let (subject, facts) = facts(notice);
    let key = notice.action.map(|(key, label)| {
        let mut line = panel::keycap(key);
        line.push(&format!(" {label}"), MUTED);
        line
    });
    fitted(glyph, Line::new(&subject, STRONG), facts, key, columns)
}

/// ` glyph label ` on the pill surface; the glyph column is two cells.
fn pill(glyph: Line, label: Line) -> Line {
    let mut line = Line::default();
    panel::push(&mut line, " ", BODY, Some(PILL_SURFACE));
    for span in glyph
        .spans
        .into_iter()
        .chain(Line::new(" ", BODY).spans)
        .chain(label.spans)
    {
        panel::push(&mut line, &span.text, span.style, Some(PILL_SURFACE));
    }
    panel::push(&mut line, " ", BODY, Some(PILL_SURFACE));
    line
}

/// The pill, then as many facts as fit with the keycap; then without it.
/// A long label yields room to the first fact down to half the row, so a
/// cause stays readable; a lone first fact is shortened rather than dropped
/// while it can be read.
fn fitted(glyph: Line, label: Line, facts: Vec<Line>, key: Option<Line>, columns: usize) -> Line {
    let joined = |kept: &[Line]| {
        let mut line = Line::default();
        for (index, fact) in kept.iter().enumerate() {
            if index > 0 {
                line.push(" · ", DIM);
            }
            line = line.append(fact.clone());
        }
        line
    };
    for key in [key.as_ref(), None]
        .into_iter()
        .take(1 + usize::from(key.is_some()))
    {
        let reserved = key.map_or(0, |key| panel::cells(key) + 2);
        let room = columns.saturating_sub(2 + reserved);
        let label_cells = panel::cells(&label);
        let wanted = facts.first().map_or(0, |first| {
            PILL_CHROME + label_cells + 2 + panel::cells(first)
        });
        let cut = wanted.saturating_sub(room);
        let kept = label_cells.saturating_sub(cut).max(LABEL_MIN.max(room / 2));
        // A cut that neither lets the fact fit nor frees a readable span
        // only costs the label its last letters.
        let worth = kept + cut <= label_cells
            || label_cells.saturating_sub(kept) >= FACT_MIN
            || PILL_CHROME + label_cells > room;
        let pill = if cut > 0 && worth {
            pill(glyph.clone(), label.shortened(kept))
        } else {
            pill(glyph.clone(), label.clone())
        };
        let used = panel::cells(&pill);
        if key.is_some() && used > room {
            continue;
        }
        let mut row = pill.clone();
        let space = room.saturating_sub(used + 2);
        let kept = (0..=facts.len())
            .rev()
            .find(|&count| count == 0 || panel::cells(&joined(&facts[..count])) <= space)
            .unwrap_or(0);
        if kept > 0 {
            row.push("  ", BODY);
            row = row.append(joined(&facts[..kept]));
        } else if let Some(first) = facts.first().filter(|_| space >= FACT_MIN) {
            row.push("  ", BODY);
            row = row.append(first.shortened(space));
        }
        if let Some(key) = key {
            let gap = columns - 2 - panel::cells(&row) - panel::cells(key);
            row.push(&" ".repeat(gap), BODY);
            row = row.append(key.clone());
        }
        return row.shortened(columns);
    }
    pill(glyph, label).shortened(columns)
}

/// `Subject · fact · fact`, or `Subject: fact`: the label and its facts.
/// A known setting change reads `old → new` with the new value strong.
fn facts(notice: &Feedback) -> (String, Vec<Line>) {
    let (subject, mut rest) = split(&notice.text);
    let value = text::clean(&notice.text);
    let colon = value[subject.len()..].starts_with(": ");
    if notice.change
        && colon
        && let Some(first) = rest.first_mut()
        && let Some((from, to)) = first.plain().split_once(" → ")
    {
        let mut line = Line::new(&format!("{from} → "), DIM);
        line.push(to, STRONG);
        *first = line;
    }
    (subject, rest)
}

/// The subject ends at the first ` · ` or `: `; the rest splits at ` · `.
fn split(value: &str) -> (String, Vec<Line>) {
    let value = text::clean(value);
    let at = [value.find(" · "), value.find(": ")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(value.len());
    let rest = value[at..]
        .strip_prefix(" · ")
        .or_else(|| value[at..].strip_prefix(": "))
        .unwrap_or("");
    let facts = rest
        .split(" · ")
        .filter(|fact| !fact.is_empty())
        .map(|fact| Line::new(fact, DIM))
        .collect();
    (value[..at].into(), facts)
}

/// `Retrying in 4s`, counting up to the next attempt.
fn countdown(until: Option<Instant>) -> String {
    let left = until.map(|until| until.saturating_duration_since(Instant::now()));
    match left.filter(|left| !left.is_zero()) {
        Some(left) => format!(
            "Retrying in {}s",
            left.as_secs() + u64::from(left.subsec_nanos() > 0)
        ),
        None => "Retrying".into(),
    }
}

/// The latest running call's `description` verbatim, else a family verb
/// and a short target; between calls, the turn's phase.
pub(super) fn verb(state: &State) -> String {
    let running = state.items.iter().rev().find_map(|item| match item {
        Item::Tool {
            name,
            arguments,
            result: None,
            ..
        } => Some((name.as_str(), arguments)),
        _ => None,
    });
    if let Some((name, arguments)) = running {
        return tool_label(name, arguments);
    }
    match state.status.trim_end_matches(['.', '…']) {
        "" | "Ready" => "Working".into(),
        phase => phase.into(),
    }
}

/// The longest target a label shows before the row shortens it.
const TARGET: usize = 32;

pub(super) fn tool_label(name: &str, arguments: &Value) -> String {
    let field = |key| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
    };
    if !field("description").is_empty() {
        return text::clean(field("description"));
    }
    let short = |value: &str| {
        let first = value
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("");
        text::ellipsize(&text::clean(first.trim()), TARGET)
    };
    let file = |key| {
        let path = field(key);
        short(
            path.rsplit(['/', '\\'])
                .find(|part| !part.is_empty())
                .unwrap_or(path),
        )
    };
    let host = |value: &str| {
        let rest = value.split_once("://").map_or(value, |(_, rest)| rest);
        short(rest.split(['/', '?', '#']).next().unwrap_or(rest))
    };
    let labelled = |verb: &str, target: String| {
        if target.is_empty() {
            verb.to_string()
        } else {
            format!("{verb} {target}")
        }
    };
    match name {
        "list" => labelled(
            "Listing",
            if field("path").is_empty() {
                ".".into()
            } else {
                file("path")
            },
        ),
        "search" => labelled("Searching", short(field("query"))),
        "read" => match field("path").strip_prefix("skill:") {
            Some(skill) => labelled(
                "Loading skill",
                short(skill.strip_suffix("/SKILL.md").unwrap_or(skill)),
            ),
            None => labelled("Reading", file("path")),
        },
        "edit" => labelled("Editing", file("path")),
        "write" => labelled("Writing", file("path")),
        "bash" => labelled("Running", short(field("command"))),
        "exec" => labelled("Starting", short(&crate::events::target(name, arguments))),
        "process" => {
            let verb = match field("action") {
                "list" => return "Listing processes".into(),
                "poll" => "Polling",
                "stop" => "Stopping",
                "write" => "Writing to",
                "resize" => "Resizing",
                "interrupt" => "Interrupting",
                _ => "Checking",
            };
            labelled(verb, short(field("process_id")))
        }
        "web" if !field("url").is_empty() => labelled("Fetching", host(field("url"))),
        "web" => "Searching the web".into(),
        "browser" if !field("url").is_empty() => labelled("Browsing", host(field("url"))),
        "browser" => "Browsing".into(),
        "checkpoint" => "Saving a checkpoint".into(),
        other => labelled("Running", short(other)),
    }
}

/// The style a test reads from the first span containing `value`.
#[cfg(test)]
pub(super) fn style_of(line: &Line, value: &str) -> Option<crate::tui::theme::Style> {
    line.spans
        .iter()
        .find(|span| span.text.contains(value))
        .map(|span| span.style)
}

#[cfg(test)]
mod tests;
