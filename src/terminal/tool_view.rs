//! Fixed-height active tools live in transient chrome, never in scrollback.
use super::{
    model::Model,
    style::{Row, Tone},
    text,
};

pub fn active(model: &Model, width: usize) -> Vec<Row> {
    let Some(group) = model.tools.active() else {
        return Vec::new();
    };
    let mut header = clipped(
        &format!("{} Exploring workspace", model.tools.marker()),
        width,
        Tone::Heading,
    );
    if header.text.len() >= model.tools.marker().len() {
        header
            .spans
            .push((0..model.tools.marker().len(), Tone::Accent));
    }
    let call = group.calls.last().unwrap();
    let target = model.blocks[call.block].text.lines().next().unwrap_or("");
    let reading = call.outcome == super::tool_activity::Outcome::Running
        && group.state == "Reading workspace";
    let detail = if reading {
        let action = match call.name {
            "read_file" => "Reading",
            "search_text" => "Searching",
            _ => "Listing",
        };
        let path = target
            .strip_prefix(call.name)
            .unwrap_or(target)
            .trim_start_matches(" / ");
        format!("  {action}  {path}")
    } else {
        format!("  {}", group.state)
    };
    let counts = format!("  {} · {}s", group.counts(), group.elapsed().as_secs());
    let has_errors = group
        .calls
        .iter()
        .any(|call| call.outcome == super::tool_activity::Outcome::Failed);
    vec![
        header,
        clipped(
            &detail,
            width,
            if reading { Tone::Accent } else { Tone::Muted },
        ),
        clipped(
            &counts,
            width,
            if has_errors { Tone::Error } else { Tone::Muted },
        ),
    ]
}

pub(super) fn clipped(value: &str, width: usize, tone: Tone) -> Row {
    let clean = text::safe(value).replace('\n', " ");
    if text::width(&clean) <= width {
        return Row::new(clean, tone);
    }
    let mut end = 0;
    let mut used = 0;
    for pair in text::boundaries(&clean).windows(2) {
        let next = used + text::width(&clean[pair[0]..pair[1]]);
        if next >= width {
            break;
        }
        used = next;
        end = pair[1];
    }
    Row::new(format!("{}…", &clean[..end]), tone)
}
