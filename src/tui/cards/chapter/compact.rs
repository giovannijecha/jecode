//! The bounded WORK view. Selection is by outcome; display stays in call order.

use super::super::{Card, Status, Tool, badge, heading};
use super::{facts::Outcomes, facts::facts, label};
use crate::tui::{
    activity as timer,
    line::Line,
    text,
    theme::{BAD, BODY, DIM, MUTED, STRONG, WARNING},
};

const MAX_ROWS: usize = 8;

pub(super) fn render(tools: &[Tool<'_>], columns: usize, keycap: bool) -> Card {
    let mut outcomes = Outcomes::default();
    for tool in tools {
        outcomes.add(Status::of(tool));
    }
    let mut count = Line::new(&text::counted(tools.len(), "call"), DIM);
    if outcomes.failed > 0 {
        count.push(&format!(" · {} failed", outcomes.failed), BAD);
    }
    let mut title = badge::badge("Work", outcomes.primary());
    title.push(" ", BODY);
    title.push(
        &tools.first().map_or_else(|| "Activity".into(), label),
        STRONG.bold(),
    );
    let title = right_aligned(title, count, columns).at_source(0);

    let width = columns.saturating_sub(4).max(2);
    let all_facts = facts(tools, &outcomes).wrap(width, true);
    let mut fact_rows: Vec<_> = all_facts.iter().take(2).cloned().collect();
    if all_facts.len() > 2 && fact_rows.len() == 2 {
        let second = fact_rows.pop().unwrap();
        let mut second = second
            .wrap(width.saturating_sub(2), false)
            .into_iter()
            .next()
            .unwrap_or_default();
        second.push(" …", DIM);
        fact_rows.push(second);
    }
    let room = MAX_ROWS.saturating_sub(2 + fact_rows.len());
    let mut priority: Vec<_> = tools
        .iter()
        .enumerate()
        .filter(|(_, tool)| Status::of(tool) != Status::Done)
        .collect();
    let attention = priority.len();
    priority.sort_by_key(|(index, tool)| (rank(Status::of(tool)), usize::MAX - *index));
    priority.truncate(room.min(3));
    priority.sort_by_key(|(index, _)| *index);
    let more_attention = attention - priority.len();
    let remaining = room.saturating_sub(priority.len() + usize::from(more_attention > 0));
    let mut recent: Vec<_> = tools
        .iter()
        .enumerate()
        .filter(|(_, tool)| Status::of(tool) == Status::Done)
        .rev()
        .take(remaining.min(4))
        .collect();
    recent.reverse();
    let hidden = tools.len() - priority.len() - recent.len();

    let mut body: Vec<_> = fact_rows;
    body.extend(priority.into_iter().map(|(_, tool)| member(tool, columns)));
    if more_attention > 0 {
        let wording = if more_attention == 1 {
            "call needs"
        } else {
            "calls need"
        };
        body.push(Line::new(
            &format!("+{more_attention} more {wording} attention"),
            WARNING,
        ));
    }
    body.extend(recent.into_iter().map(|(_, tool)| member(tool, columns)));
    let mut rows = vec![title];
    rows.extend(body.into_iter().enumerate().map(|(index, line)| {
        line.shortened(width)
            .indent("  │ ", DIM)
            .at_source(index + 1)
    }));

    let mut footer = Line::new(&format!("  └ {hidden} calls folded"), DIM);
    if keycap {
        let mut cap = Line::new("Ctrl+O", STRONG.bold());
        if columns >= 19 {
            cap.push(" expand", DIM);
        }
        footer = right_aligned(footer, cap, columns);
    }
    rows.push(footer.shortened(columns).at_source(rows.len()));
    debug_assert!(rows.len() <= MAX_ROWS);
    Card { rows, hidden: true }
}

fn rank(status: Status) -> u8 {
    match status {
        Status::Failed => 0,
        Status::Unknown => 1,
        Status::Stopped => 2,
        Status::Running => 3,
        Status::Background => 4,
        Status::Done => 5,
    }
}

fn member(tool: &Tool<'_>, columns: usize) -> Line {
    let status = Status::of(tool);
    let detail = match status {
        Status::Failed => tool
            .result
            .and_then(|result| result.get("exit_code"))
            .filter(|value| **value != crate::json::Value::Null)
            .map_or_else(
                || "failed".into(),
                |value| format!("exit {}", value.encode()),
            ),
        Status::Unknown => "outcome unknown".into(),
        Status::Stopped => "stopped".into(),
        Status::Running => format!(
            "Running · {}",
            timer::duration(tool.presentation.duration().unwrap_or_default())
        ),
        Status::Background => "background".into(),
        Status::Done => tool
            .presentation
            .duration()
            .map_or_else(String::new, timer::duration),
    };
    let tone = match status {
        Status::Failed => BAD,
        Status::Unknown | Status::Stopped => WARNING,
        _ => DIM,
    };
    let detail = Line::new(&detail, tone);
    let (name, target) = heading(tool);
    let mut left = Line::new(&name.to_uppercase(), badge::ink(name).bold());
    left.push(" ", BODY);
    let description = super::super::rhythm::description(tool);
    let target = if tool.name == "collaboration" {
        tool.result
            .map_or_else(String::new, super::super::agents::subject)
    } else if description.is_empty() {
        text::clean(&target)
    } else {
        text::clean(description)
    };
    let target = super::super::rhythm::inline(&target);
    let width = columns
        .saturating_sub(8 + text::cells(&left.plain()) + text::cells(&detail.plain()))
        .max(1);
    left.push(&middle(&target, width), BODY);
    right_aligned(left, detail, columns.saturating_sub(4))
}

fn middle(value: &str, width: usize) -> String {
    if text::cells(value) <= width {
        return value.into();
    }
    let glyphs: Vec<_> = text::glyphs(value).collect();
    let head_room = (width.saturating_sub(1) * 3).div_ceil(5);
    let tail_room = width.saturating_sub(1 + head_room);
    let mut first = String::new();
    let mut used = 0;
    for glyph in &glyphs {
        if used + glyph.cells > head_room {
            break;
        }
        first.push_str(glyph.text);
        used += glyph.cells;
    }
    let mut last = String::new();
    used = 0;
    for glyph in glyphs.iter().rev() {
        if used + glyph.cells > tail_room {
            break;
        }
        last.insert_str(0, glyph.text);
        used += glyph.cells;
    }
    format!("{first}…{last}")
}

fn right_aligned(left: Line, right: Line, columns: usize) -> Line {
    let end = columns.saturating_sub(2);
    let right_width = text::cells(&right.plain());
    let room = end.saturating_sub(right_width + 2);
    let mut left = left.shortened(room);
    let gap = end.saturating_sub(text::cells(&left.plain()) + right_width);
    left.push(&" ".repeat(gap), MUTED);
    left.append(right).shortened(columns)
}
