//! A pure view of the real controller. Typed rows own no session or effects.
use super::{lab, model::Model};
use lab::{activity_view, composer, model, picker, style};

pub(super) struct Snapshot {
    blocks: Vec<model::Block>,
    live: Option<String>,
    activity: Option<activity_view::Activity>,
    notice: Option<model::Notice>,
    draft: String,
    cursor: usize,
    suggestion: usize,
    footer: composer::Footer,
    picker: Option<picker::Picker>,
    expanded: bool,
    queued: Vec<String>,
}

impl Snapshot {
    pub fn from_model(model: &Model) -> Self {
        let account = model.account.as_ref();
        let mut blocks = Vec::with_capacity(model.blocks.len());
        let mut live = None;
        for (index, block) in model.blocks.iter().enumerate() {
            if let Some(receipt) = model.command_receipts.get(&index) {
                blocks.push(model::Block::Command(receipt.clone()));
                continue;
            }
            if let Some(tool) = model.tool_details.get(&index) {
                match blocks.last_mut() {
                    Some(model::Block::Tools(tools)) => tools.push(tool.clone()),
                    _ => blocks.push(model::Block::Tools(vec![tool.clone()])),
                }
                continue;
            }
            match block.speaker {
                "ToolSummary" | "ToolWarning" => {}
                "You" => blocks.push(model::Block::User(block.text.clone())),
                "Assistant" | "Demo" | "Demo / failure" | "Correction" => {
                    if model.streaming() && index + 1 == model.blocks.len() {
                        live = Some(block.text.clone());
                    } else if !block.text.is_empty() {
                        blocks.push(model::Block::Assistant(block.text.clone()));
                    }
                }
                "Tool" | "Command" | "Edit" | "CommandPreview" | "EditPreview" => {
                    let tool = fallback_tool(block.speaker, &block.text);
                    match blocks.last_mut() {
                        Some(model::Block::Tools(tools)) => tools.push(tool),
                        _ => blocks.push(model::Block::Tools(vec![tool])),
                    }
                }
                other => blocks.push(model::Block::Local {
                    text: block.text.clone(),
                    failed: other == "Error",
                }),
            }
        }
        let activity = model.streaming().then(|| {
            let phase = if model.tools.active().is_some() || model.action_demo.is_some() {
                activity_view::Phase::Working
            } else {
                match account
                    .map(|view| view.notice.as_str())
                    .unwrap_or(model.status)
                {
                    text if text.contains("Think") => activity_view::Phase::Thinking,
                    text if text.contains("Stream") => activity_view::Phase::Streaming,
                    text if text.contains("command")
                        || text.contains("file")
                        || text.contains("workspace")
                        || text.contains("tool") =>
                    {
                        activity_view::Phase::Working
                    }
                    _ => activity_view::Phase::Waiting,
                }
            };
            activity_view::Activity {
                phase,
                elapsed_ms: account.map_or(0, super::account::View::elapsed_ms),
                // A stream has no measured token count until provider usage arrives.
                tokens: 0,
            }
        });
        let notice = account
            .and_then(|view| {
                let text = if !view.local_notice.is_empty() {
                    view.local_notice.as_str()
                } else if !view.generating() && !view.notice.is_empty()
                    || view.notice.contains("retry")
                    || view.notice.contains("Stopping")
                {
                    view.notice.as_str()
                } else {
                    ""
                };
                (!text.is_empty()).then(|| model::Notice {
                    status: if view.failed || view.local_failed {
                        model::Status::Failed
                    } else if view.local_notice.is_empty() && text.starts_with("Complete") {
                        model::Status::Done
                    } else {
                        model::Status::Warned
                    },
                    text: text.into(),
                })
            })
            .or_else(|| {
                (!model.edit_notice.is_empty()).then(|| model::Notice {
                    status: model::Status::Failed,
                    text: model.edit_notice.into(),
                })
            });
        let footer = account.map_or_else(
            || composer::Footer {
                cwd: "Local demo".into(),
                model: "offline".into(),
                effort: "preview".into(),
            },
            |view| composer::Footer {
                cwd: view
                    .directory
                    .clone()
                    .unwrap_or_else(|| "Conversation only".into()),
                model: view.selected.id().into(),
                effort: view.selected.effort().unwrap_or("provider default").into(),
            },
        );
        let picker = model.menu.panel.as_ref().map(|panel| picker::Picker {
            title: panel.title.into(),
            choices: panel
                .entries
                .iter()
                .map(|entry| picker::Choice {
                    label: entry.label.clone(),
                    detail: entry.description.clone(),
                })
                .collect(),
            query: model.menu.query.clone(),
            selected: model.menu.selected,
        });
        Self {
            blocks,
            live,
            activity,
            notice,
            draft: model.editor.text.clone(),
            cursor: model.editor.cursor,
            suggestion: if model.menu.active(&model.editor.text) {
                model.menu.selected
            } else {
                usize::MAX
            },
            footer,
            picker,
            expanded: model.expanded,
            queued: account.map_or_else(
                || model.demo_queue.iter().cloned().collect(),
                super::account::View::pending_messages,
            ),
        }
    }

    pub fn frame(
        &self,
        layout: &mut lab::view::Layout,
        width: usize,
        height: usize,
        caps: &lab::caps::Caps,
        now_ms: u64,
    ) -> Vec<style::Row> {
        layout.frame_bounded(
            &lab::view::Screen {
                blocks: &self.blocks,
                live: self.live.as_deref(),
                activity: self.activity,
                notice: self.notice.as_ref(),
                draft: &self.draft,
                cursor: self.cursor,
                suggestion: self.suggestion,
                footer: &self.footer,
                picker: self.picker.as_ref(),
                expanded: self.expanded,
                queued: &self.queued,
            },
            width,
            height,
            caps,
            now_ms,
        )
    }

    pub fn preview(
        &self,
        width: usize,
        height: usize,
        caps: &lab::caps::Caps,
        now_ms: u64,
    ) -> Vec<style::Row> {
        lab::view::preview(
            &lab::view::Screen {
                blocks: &self.blocks,
                live: self.live.as_deref(),
                activity: self.activity,
                notice: self.notice.as_ref(),
                draft: &self.draft,
                cursor: self.cursor,
                suggestion: self.suggestion,
                footer: &self.footer,
                picker: self.picker.as_ref(),
                expanded: self.expanded,
                queued: &self.queued,
            },
            width,
            height,
            caps,
            now_ms,
        )
    }
}

fn fallback_tool(role: &str, text: &str) -> model::Tool {
    let (verb, detail) = if role.starts_with("Edit") {
        ("Edit", model::Detail::Diff(text.into()))
    } else {
        (
            if role.starts_with("Command") {
                "Run"
            } else {
                "Tool"
            },
            model::Detail::Output(String::new()),
        )
    };
    model::Tool {
        verb: verb.into(),
        subject: text.lines().next().unwrap_or("").into(),
        summary: "historical detail unavailable".into(),
        status: model::Status::Warned,
        elapsed_ms: 0,
        detail,
    }
}
