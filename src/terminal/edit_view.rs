//! Live file activity and exact result, without a decision surface.
use super::model::{Block, Model};
use crate::workspace::Preview;

pub(super) struct Edit {
    id: u64,
    block: usize,
}

pub(super) fn planned(model: &mut Model, id: u64, preview: Preview) {
    model
        .tools
        .close(&model.blocks, false, std::time::Instant::now());
    let block = model.blocks.len();
    let legend = if preview.diff.contains("\\\\")
        || preview.diff.contains("\\t")
        || preview.diff.contains("\\r")
    {
        "  Diff escapes: \\\\ = backslash, \\t = tab, \\r = CR\n"
    } else {
        ""
    };
    let omitted = if preview.omitted_bytes > 0 {
        format!(
            "\n  ... {} diff lines / {} bytes omitted",
            preview.omitted_lines, preview.omitted_bytes
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
    if let Some(view) = &mut model.account {
        view.edit = Some(Edit { id, block });
        view.notice = "Applying file change".into();
    }
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
    if view.edit.as_ref().is_some_and(|edit| edit.id == id) {
        let edit = view.edit.take().unwrap();
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

pub(super) fn stop(model: &mut Model) {
    if let Some(edit) = model.account.as_mut().and_then(|view| view.edit.take()) {
        model.blocks[edit.block].text.push_str("\n! Turn stopped before the file result was received; inspect the file before repeating the change");
    }
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
