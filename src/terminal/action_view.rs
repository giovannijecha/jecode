//! Inert activity and receipt styling. No authority to perform effects.
use super::{
    action_demo::{Demo, Kind},
    model::Block,
    style::{Row, Tone, lines, pad},
    tool_view::clipped,
};

pub fn active(demo: &Demo, width: usize, reduced_motion: bool) -> Vec<Row> {
    let marker = demo.spinner.marker(reduced_motion);
    let action = if demo.kind == Kind::Edit {
        "Updating file"
    } else {
        "Running command"
    };
    let header = super::tool_view::indicator(marker, action, width);
    vec![
        header,
        clipped(
            &format!("  {:.1}s · local preview", demo.elapsed().as_secs_f64()),
            width,
            Tone::Muted,
        ),
    ]
}

/// Preview text is local and bounded, but still uses the ordinary terminal sanitizer.
/// Diff signs and result words remain readable without color.
pub fn receipt(block: &Block, width: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    let edit = matches!(block.speaker, "EditPreview" | "Edit");
    let mut previous_panel = false;
    let mut had_panel = false;
    for (index, source) in block.text.lines().enumerate() {
        // Real process output cannot forge a receipt or error heading.
        let output = (block.speaker == "Command")
            .then(|| source.strip_prefix("| "))
            .flatten();
        let rendered_output = output.map(|text| format!("  {text}"));
        let line = rendered_output.as_deref().unwrap_or(source);
        let tone = if output.is_some() {
            Tone::Code
        } else if line.starts_with('!') {
            Tone::Error
        } else if line.starts_with('✓') || index == 0 {
            Tone::Heading
        } else if edit && line.starts_with('-') {
            Tone::Removed
        } else if edit && line.starts_with('+') {
            Tone::Added
        } else if line.starts_with("  $") {
            Tone::Accent
        } else if line.starts_with("  cwd:")
            || line.starts_with("  shell:")
            || line.starts_with("  timeout:")
            || line.starts_with("  Diff escapes:")
            || line.starts_with("  Command escapes:")
            || line.starts_with("  Recovery:")
            || line.starts_with("  Approved")
            || line.starts_with('·')
            || line.starts_with("  @@")
        {
            Tone::Muted
        } else {
            Tone::Code
        };
        let panel = matches!(tone, Tone::Added | Tone::Removed | Tone::Code);
        let outcome = line.starts_with(['✓', '!', '·']);
        // Keep metadata and output lines compact, with one separator at the
        // panel boundary. A command without output still has an outcome row.
        if index > 0 && (panel != previous_panel || outcome && !had_panel) {
            rows.push(Row::blank());
        }
        previous_panel = panel;
        had_panel |= panel;
        let mut rendered = lines(line, width, tone);
        if line.starts_with('✓')
            && let Some(row) = rendered.first_mut()
        {
            row.spans.push((0..'✓'.len_utf8(), Tone::Success));
        }
        for row in rendered {
            rows.push(if panel { pad(row, width) } else { row });
        }
    }
    rows
}
