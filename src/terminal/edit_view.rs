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
    model.tool_details.insert(
        block,
        super::lab::model::Tool {
            verb: if preview.create { "Create" } else { "Edit" }.into(),
            subject: preview.path.clone(),
            summary: format!(
                "+{} -{}{}",
                preview.added,
                preview.removed,
                if preview.omitted_bytes > 0 {
                    " / preview truncated"
                } else {
                    ""
                }
            ),
            status: super::lab::model::Status::Running,
            elapsed_ms: 0,
            detail: super::lab::model::Detail::Diff(preview.diff.clone()),
        },
    );
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
        if let Some(tool) = model.tool_details.get_mut(&edit.block) {
            tool.status = if failed {
                super::lab::model::Status::Failed
            } else if applied {
                super::lab::model::Status::Done
            } else {
                super::lab::model::Status::Warned
            };
            tool.summary = summary.clone();
        }
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
        if let Some(tool) = model.tool_details.get_mut(&edit.block) {
            tool.status = super::lab::model::Status::Warned;
            tool.summary = "outcome uncertain; inspect file".into();
        }
        model.blocks[edit.block].text.push_str("\n! Turn stopped before the file result was received; inspect the file before repeating the change");
    }
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
