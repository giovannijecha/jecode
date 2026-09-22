//! Translate conversation events to presentation; no network or credential owner.
use super::{
    Key,
    model::{Block, Model},
};
use crate::{
    providers::openai_account::auth,
    session::{self, End, Event, Metrics, Session},
};
use std::time::Instant;

#[derive(PartialEq)]
enum Phase {
    Login,
    Ready,
    Generating,
    Updating,
    Closed,
}
enum LocalOperation {
    Model,
    Context,
    Compact(String),
}
pub(super) struct View {
    phase: Phase,
    pub notice: String,
    pub failed: bool,
    turns: usize,
    pub queued: usize,
    active_tool: Option<usize>,
    pub approval: Option<super::approval_view::Approval>,
    pub command: Option<super::command_view::Run>,
    pub selected: session::Model,
    pub id: Option<String>,
    pub workspace: Option<String>,
    pub access: crate::workspace::Access,
    local_operation: Option<LocalOperation>,
}
impl View {
    pub fn ready(&self) -> bool {
        self.phase == Phase::Ready
    }
    pub fn inspecting(&mut self) {
        self.phase = Phase::Updating;
        self.local_operation = Some(LocalOperation::Context);
        self.notice = "Measuring context…".into();
    }
    pub fn generating(&self) -> bool {
        self.phase == Phase::Generating
    }
    pub fn updating(&mut self) {
        self.phase = Phase::Updating;
        self.local_operation = Some(LocalOperation::Model);
        self.notice = "Changing model…".into();
    }
}
pub(super) fn model(selected: session::Model, workspace: Option<&std::path::Path>) -> Model {
    let mut model = Model::new(Instant::now());
    model.blocks.clear();
    let workspace = workspace.map(|path| {
        let path = path.to_string_lossy();
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    });
    model.account = Some(View {
        phase: Phase::Login,
        notice: "Connecting for sign-in / Esc cancels".into(),
        failed: false,
        turns: 0,
        queued: 0,
        active_tool: None,
        approval: None,
        command: None,
        selected,
        id: None,
        workspace,
        access: crate::workspace::Access::Workspace,
        local_operation: None,
    });
    model
}

pub(super) fn input(model: &mut Model, key: Key, session: &mut Session) {
    if super::approval_view::input(model, &key, session) {
        return;
    }
    if super::commands::input(model, &key, session) {
        return;
    }
    let Some(view) = &mut model.account else {
        return;
    };
    match key {
        Key::Enter => {
            if view.phase == Phase::Generating && !model.editor.text.trim().is_empty() {
                if session.enqueue(&model.editor.text) {
                    model.editor.take();
                    view.queued += 1;
                } else {
                    view.notice = "Queue full or stopping / draft kept".into();
                }
                return;
            }
            if view.phase != Phase::Ready || model.editor.text.trim().is_empty() {
                return;
            }
            if view.turns >= 256 {
                view.notice = "Session limit reached / draft kept; restart to continue".into();
                view.failed = true;
                return;
            }
            if !session.submit(&model.editor.text) {
                view.notice = "Message not sent / draft kept".into();
                view.failed = true;
                return;
            }
            view.turns += 1;
            model.blocks.push(Block {
                speaker: "You",
                text: model.editor.take(),
            });
            model.blocks.push(Block {
                speaker: "Assistant",
                text: String::new(),
            });
            view.phase = Phase::Generating;
            view.failed = false;
            view.notice = "Waiting for model / Esc stops".into();
        }
        Key::Escape | Key::Interrupt if matches!(view.phase, Phase::Login | Phase::Generating) => {
            session.cancel();
            view.notice = "Stopping...".into();
            if let Some(command) = &mut view.command {
                command.stopping = true;
            }
            model.tools.waiting("Stopping / waiting for cleanup");
        }
        Key::Escape => {}
        key => model.input(key, Instant::now()),
    }
}

pub(super) fn event(model: &mut Model, event: Event) {
    if matches!(event, Event::Finished(..) | Event::LoginFailed(_)) {
        super::approval_view::stop(model);
        super::command_view::stop(model);
    }
    let Some(view) = &mut model.account else {
        return;
    };
    match event {
        Event::Guidance { text, new_turn } => {
            view.queued = view.queued.saturating_sub(1);
            if new_turn {
                view.turns += 1;
            }
            view.phase = Phase::Generating;
            model.tools.close(&model.blocks, false, Instant::now());
            model.blocks.push(Block {
                speaker: "You",
                text,
            });
            model.blocks.push(Block {
                speaker: "Assistant",
                text: String::new(),
            });
        }
        Event::GuidanceReturned(text) => {
            view.queued = view.queued.saturating_sub(1);
            model.blocks.push(Block {
                speaker: "Status",
                text: format!("Queued message was not sent:\n{text}"),
            });
        }
        Event::ContextReport(text) => {
            if matches!(view.local_operation, Some(LocalOperation::Context)) {
                view.local_operation = None;
                view.phase = Phase::Ready;
                view.notice.clear();
                model.blocks.push(Block {
                    speaker: "Status",
                    text,
                });
            } else if let Some(LocalOperation::Compact(report)) = &mut view.local_operation {
                report.clone_from(&text);
                view.notice = text;
            } else if view.phase == Phase::Generating {
                view.notice = text;
            } else {
                model.blocks.push(Block {
                    speaker: "Status",
                    text,
                });
            }
        }
        Event::Restored { id, items, turns } => {
            view.turns = turns;
            view.id = Some(id);
            model.blocks.extend(items.into_iter().map(|item| Block {
                speaker: item.role,
                text: item.text,
            }));
        }
        Event::EditProposed { id, preview } if view.phase == Phase::Generating => {
            super::approval_view::proposal(model, id, preview)
        }
        Event::EditFinished {
            id,
            summary,
            applied,
            failed,
        } => super::approval_view::finished(model, id, summary, applied, failed),
        Event::CommandProposed { id, preview } if view.phase == Phase::Generating => {
            super::command_view::proposal(model, id, preview)
        }
        Event::CommandStarted { id } => super::command_view::started(model, id),
        Event::CommandOutput { id, channel, text } => {
            super::command_view::output(model, id, channel, &text)
        }
        Event::CommandFinished {
            id,
            summary,
            success,
            failed,
        } => super::command_view::finished(model, id, summary, success, failed),
        Event::LoginCode(code) => {
            view.notice = format!(
                "Sign in at {}\nEnter code: {code}\nWaiting for approval / Esc cancels",
                auth::VERIFICATION_URL
            );
        }
        Event::Ready => {
            view.phase = Phase::Ready;
            view.notice.clear();
        }
        Event::ModelChanged(selected) => {
            view.selected = selected;
            view.phase = Phase::Ready;
            view.local_operation = None;
            view.notice.clear();
        }
        Event::Thinking if view.phase == Phase::Generating => {
            view.notice = "Thinking / Esc stops".into();
            model.tools.waiting("Thinking");
        }
        Event::RequestStarted if view.phase == Phase::Generating => {
            model.blocks.push(Block {
                speaker: "Assistant",
                text: String::new(),
            });
            view.notice = "Waiting for model / Esc stops".into();
            model.tools.waiting("Waiting for model");
        }
        Event::ToolStarted { name, path } if view.phase == Phase::Generating => {
            view.notice = "Reading workspace / Esc stops".into();
            let index = model.start_tool(name, path, Instant::now());
            model.account.as_mut().unwrap().active_tool = Some(index);
        }
        Event::ToolFinished {
            summary,
            failed,
            limited,
        } if view.phase == Phase::Generating => {
            if let Some(index) = view.active_tool.take() {
                model.finish_tool(index, &summary, failed, limited, Instant::now());
            }
        }
        Event::Text(text) if view.phase == Phase::Generating => {
            if !text.is_empty() {
                model.tools.close(&model.blocks, false, Instant::now());
            }
            if let Some(block) = model.blocks.last_mut()
                && block.speaker == "Assistant"
            {
                block.text.push_str(&text);
            }
            view.notice = "Streaming / Esc stops".into();
        }
        Event::Finished(end, metrics) => {
            model.tools.close(
                &model.blocks,
                !matches!(end, End::Complete | End::Refused),
                Instant::now(),
            );
            view.phase = if end == End::Failed(session::Failure::Storage) {
                Phase::Closed
            } else {
                Phase::Ready
            };
            view.failed = matches!(end, End::Failed(_));
            view.notice = completion(end, metrics);
            if let Some(operation) = view.local_operation.take() {
                let text = match operation {
                    LocalOperation::Compact(report)
                        if end == End::Complete && !report.is_empty() =>
                    {
                        report
                    }
                    _ => view.notice.clone(),
                };
                model.blocks.push(Block {
                    speaker: if view.failed { "Error" } else { "Status" },
                    text,
                });
                view.notice.clear();
                view.failed = false;
            } else if !matches!(end, End::Complete | End::Refused) {
                // The visible attempt remains in scrollback. Its unfinished text
                // is never invented as a completed assistant item in model input.
                model.blocks.push(Block {
                    speaker: if view.failed { "Error" } else { "Status" },
                    text: std::mem::take(&mut view.notice),
                });
                view.failed = false;
            }
            if view.phase == Phase::Closed {
                view.notice =
                    "Session stopped / check local storage before resuming / Ctrl+Q exits".into();
            }
        }
        Event::LoginFailed(failure) => {
            model.tools.close(&model.blocks, true, Instant::now());
            view.phase = Phase::Closed;
            view.failed = true;
            view.notice = format!("{failure}\nRestart --account to sign in / Ctrl+Q exits");
        }
        Event::Thinking
        | Event::EditProposed { .. }
        | Event::CommandProposed { .. }
        | Event::Text(_)
        | Event::RequestStarted
        | Event::ToolStarted { .. }
        | Event::ToolFinished { .. } => {}
    }
}
pub(super) fn compacting(model: &mut Model) {
    if let Some(view) = &mut model.account {
        view.phase = Phase::Generating;
        view.local_operation = Some(LocalOperation::Compact(String::new()));
        view.notice = "Compacting context / Esc stops".into();
    }
}
fn completion(end: End, metrics: Metrics) -> String {
    let mut result = match end {
        End::Complete => "Complete".into(),
        End::Refused => "Response refused".into(),
        End::Incomplete => "Incomplete response / partial output retained".into(),
        End::Failed(failure) => failure.to_string(),
    };
    result.push_str(&format!(" / {:.1}s", metrics.elapsed_ms as f64 / 1000.0));
    if metrics.approval_wait_ms >= 1000 {
        result.push_str(&format!(
            " ({:.1}s awaiting approval)",
            metrics.approval_wait_ms as f64 / 1000.0
        ));
    }
    if metrics.tool_calls != 0 {
        result.push_str(&format!(
            " / {} tools / {} requests",
            metrics.tool_calls, metrics.requests
        ));
    }
    if let (Some(input), Some(output)) = (metrics.input_tokens, metrics.output_tokens) {
        result.push_str(&format!(" / tokens: {input} in, {output} out"));
    }
    result
}

#[cfg(test)]
#[path = "account_tests.rs"]
mod tests;
