//! Shared decision surface. Only Session decides and executes held proposals.
use super::{
    Key,
    model::{Block, Model},
};
use crate::{session::Session, workspace::Preview};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Edit,
    Command,
}
pub(super) struct Approval {
    pub id: u64,
    pub block: usize,
    pub allow: bool,
    pub submitted: bool,
    pub displayed: bool,
    pub kind: Kind,
}
pub(super) fn proposal(model: &mut Model, id: u64, preview: Preview) {
    model
        .tools
        .close(&model.blocks, false, std::time::Instant::now());
    let block = model.blocks.len();
    let legend = if preview.full_diff_path.is_some()
        || preview.diff.contains("\\\\")
        || preview.diff.contains("\\t")
        || preview.diff.contains("\\r")
    {
        "  Diff escapes: \\\\ = backslash, \\t = tab, \\r = CR\n"
    } else {
        ""
    };
    let omitted = if let Some(path) = &preview.full_diff_path {
        format!(
            "\n  ... {} diff lines / {} bytes omitted. Full diff while approval waits: {}",
            preview.omitted_lines, preview.omitted_bytes, path
        )
    } else {
        String::new()
    };
    model.blocks.push(Block {
        speaker: "Edit",
        text: format!(
            "  {} {} · +{} -{}\n{legend}{}{omitted}",
            if preview.create { "Create" } else { "Edit" },
            preview.path,
            preview.added,
            preview.removed,
            preview.diff.trim_end_matches('\n')
        ),
    });
    open(model, id, block, Kind::Edit);
}
pub(super) fn open(model: &mut Model, id: u64, block: usize, kind: Kind) {
    if let Some(view) = &mut model.account {
        view.approval = Some(Approval {
            id,
            block,
            allow: false,
            submitted: false,
            displayed: false,
            kind,
        });
        view.notice = "Waiting for your decision".into();
    }
}
pub(super) fn input(model: &mut Model, key: &Key, session: &mut Session) -> bool {
    let Some(view) = &mut model.account else {
        return false;
    };
    let Some(edit) = &mut view.approval else {
        return false;
    };
    if edit.submitted || *key == Key::Quit {
        return false;
    }
    if !edit.displayed && !matches!(key, Key::Escape | Key::Interrupt) {
        return true;
    }
    let allow = match key {
        Key::Left => {
            edit.allow = false;
            return true;
        }
        Key::Right => {
            edit.allow = true;
            return true;
        }
        Key::Enter => edit.allow,
        Key::Escape => false,
        Key::Interrupt => {
            session.cancel();
            edit.submitted = true;
            view.notice = "Stopping...".into();
            return true;
        }
        _ => return true, // Typing/paste never confirms an approval or alters its draft.
    };
    if session.decide(edit.id, allow) {
        edit.submitted = true;
        view.notice = if allow {
            if edit.kind == Kind::Edit {
                "Applying approved change"
            } else {
                "Starting approved command"
            }
        } else {
            "Recording denial"
        }
        .into();
        if allow {
            model.blocks[edit.block].text.push_str("\n  Approved once");
        }
    } else {
        view.notice = "Decision expired / waiting for the controller".into();
    }
    true
}
pub(super) fn finished(model: &mut Model, id: u64, summary: String, applied: bool, failed: bool) {
    model
        .tools
        .close(&model.blocks, false, std::time::Instant::now());
    let Some(view) = &mut model.account else {
        return;
    };
    let marker = if applied {
        "✓"
    } else if failed {
        "!"
    } else {
        "·"
    };
    let text = format!("{marker} {summary}");
    if view.approval.as_ref().is_some_and(|edit| edit.id == id) {
        let edit = view.approval.take().unwrap();
        model.blocks[edit.block].text.push('\n');
        model.blocks[edit.block].text.push_str(&text);
    } else {
        model.blocks.push(Block {
            speaker: if failed { "Error" } else { "Status" },
            text,
        });
    }
    view.notice = "Processing tool result".into();
}
pub(super) fn displayed(model: &mut Model, size: (usize, usize), transcript: bool) {
    if let Some(edit) = model.account.as_mut().and_then(|v| v.approval.as_mut()) {
        if size.0 < 25 || size.1 < 9 {
            edit.displayed = false;
        } else if transcript {
            edit.displayed = true;
        }
    }
}
pub(super) fn stop(model: &mut Model) {
    if let Some(edit) = model.account.as_mut().and_then(|v| v.approval.take()) {
        model.blocks[edit.block]
            .text
            .push_str("\n! Turn stopped before the action result was received");
    }
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
