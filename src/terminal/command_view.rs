//! A live command panel; output is data and can never become UI control sequences.
use super::{
    model::{Block, Model},
    spinner::Spinner,
    style::{Row, Tone},
    tool_view::clipped,
};
use crate::command::{Channel, Preview};
use std::time::Instant;

pub(super) struct Run {
    id: u64,
    block: usize,
    started: Instant,
    channel: Option<Channel>,
    line_open: bool,
    pub spinner: Spinner,
    pub stopping: bool,
    running: bool,
}
pub(super) fn planned(model: &mut Model, id: u64, preview: Preview) {
    model.tools.close(&model.blocks, false, Instant::now());
    let block = model.blocks.len();
    let escaped = preview.command.contains('\t');
    let visible = if escaped {
        preview.command.replace('\\', "\\\\").replace('\t', "\\t")
    } else {
        preview.command
    };
    let legend = if escaped {
        "  Command escapes: \\\\ = backslash, \\t = tab\n"
    } else {
        ""
    };
    let script = visible
        .lines()
        .map(|line| format!("  $ {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    model.blocks.push(Block {
        speaker: "Command",
        text: format!(
            "  Run command\n{legend}{script}\n  cwd: {}\n  shell: {}\n  timeout: {}s · stdin closed",
            preview.cwd, preview.shell, preview.timeout_seconds
        ),
    });
    if let Some(view) = &mut model.account {
        view.command = Some(Run {
            id,
            block,
            started: Instant::now(),
            spinner: Spinner::default(),
            channel: None,
            line_open: false,
            stopping: false,
            running: false,
        });
        view.notice = "Starting command".into();
    }
}
pub(super) fn started(model: &mut Model, id: u64) {
    let Some(view) = &mut model.account else {
        return;
    };
    let Some(run) = view.command.as_mut().filter(|run| run.id == id) else {
        return;
    };
    let started = Instant::now();
    run.started = started;
    run.spinner.reset(started);
    run.running = true;
    view.notice = "Running command".into();
}
pub(super) fn output(model: &mut Model, id: u64, channel: Channel, text: &str) {
    if model
        .account
        .as_ref()
        .and_then(|v| v.command.as_ref())
        .is_some_and(|run| run.id == id && !run.running)
    {
        // An output event proves launch even if the transient start event was full.
        started(model, id);
    }
    let Some(run) = model
        .account
        .as_mut()
        .and_then(|v| v.command.as_mut())
        .filter(|r| r.id == id)
    else {
        return;
    };
    let block = &mut model.blocks[run.block];
    for part in text.split_inclusive('\n') {
        if !run.line_open || run.channel != Some(channel) {
            if !block.text.ends_with('\n') {
                block.text.push('\n');
            }
            block.text.push_str(if channel == Channel::Stderr {
                "| stderr: "
            } else {
                "| "
            });
        }
        block.text.push_str(part);
        run.line_open = !part.ends_with('\n');
        run.channel = Some(channel);
    }
}
pub(super) fn finished(model: &mut Model, id: u64, summary: String, success: bool, failed: bool) {
    model.tools.close(&model.blocks, false, Instant::now());
    let Some(view) = &mut model.account else {
        return;
    };
    if view.command.as_ref().is_some_and(|run| run.id == id) {
        let run = view.command.take().unwrap();
        let block = &mut model.blocks[run.block];
        if !block.text.ends_with('\n') {
            block.text.push('\n');
        }
        block
            .text
            .push_str(&format!("{} {summary}", if success { "✓" } else { "!" }));
        view.notice = "Processing command result".into();
    } else {
        model.blocks.push(Block {
            speaker: if failed { "Error" } else { "Status" },
            text: format!("{} {summary}", if success { "✓" } else { "!" }),
        });
        view.notice = "Processing command result".into();
    }
}
pub(super) fn active(run: &Run, width: usize, reduced: bool) -> Vec<Row> {
    let marker = run.spinner.marker(reduced);
    let state = if run.stopping {
        "Stopping command"
    } else if !run.running {
        "Starting command"
    } else {
        "Running command"
    };
    let header = super::tool_view::indicator(marker, state, width);
    let elapsed = format!("  {:.1}s", run.started.elapsed().as_secs_f64());
    let detail = if run.stopping {
        format!("{elapsed} · waiting for cleanup")
    } else {
        elapsed
    };
    vec![header, clipped(&detail, width, Tone::Muted)]
}
pub(super) fn stop(model: &mut Model) {
    if let Some(run) = model.account.as_mut().and_then(|v| v.command.take()) {
        model.blocks[run.block].text.push_str("\n! Turn stopped before the command result was received; inspect effects before repeating it");
    }
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
