//! Translate conversation events to presentation; no network or credential owner.
use super::{
    Key,
    model::{Block, Model},
};
use crate::{
    providers::openai_account::auth,
    session::{self, End, Event, Metrics, Session},
};
use std::sync::Arc;
use std::time::Instant;

#[derive(PartialEq)]
enum Phase {
    Login,
    SignedOut,
    SigningOut,
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
pub(super) const UNAVAILABLE_SELECTION_NOTICE: &str =
    "Selected model or effort unavailable in this account catalog / use /model; draft kept";
pub(super) struct View {
    phase: Phase,
    pub notice: String,
    pub local_notice: String,
    pub local_failed: bool,
    pub failed: bool,
    pub partial_output: bool,
    turns: usize,
    pub pending: Option<Arc<session::PendingGuidance>>,
    pub recovery: Option<super::recovery::SavedDraft>,
    active_tool: Option<usize>,
    pub edit: Option<super::edit_view::Edit>,
    pub command: Option<super::command_view::Run>,
    pub selected: session::Model,
    pub catalog: Option<crate::providers::openai_account::catalog::Catalog>,
    pub pending_catalog: Option<bool>, // true: selecting defaults
    pub id: Option<String>,
    pub directory: Option<String>,
    pub file_tools: bool,
    pub access: crate::workspace::Access,
    local_operation: Option<LocalOperation>,
}
impl View {
    pub fn ready(&self) -> bool {
        self.phase == Phase::Ready
    }
    pub fn signed_out(&self) -> bool {
        self.phase == Phase::SignedOut
    }
    pub fn logging_in(&mut self) {
        self.phase = Phase::Login;
        self.failed = false;
        self.notice = "Connecting for sign-in / Esc cancels".into();
    }
    pub fn signing_out(&mut self) {
        self.phase = Phase::SigningOut;
        self.failed = false;
        self.notice = "Signing out / stopping active work".into();
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
    pub fn pending_messages(&self) -> Vec<String> {
        self.pending
            .as_ref()
            .map_or_else(Vec::new, |queue| queue.snapshot())
    }
    pub fn loading_catalog(&mut self, defaults: bool) {
        self.phase = Phase::Updating;
        self.pending_catalog = Some(defaults);
        self.notice = "Loading account models / Esc cancels".into();
    }
}
pub(super) fn model(selected: session::Model, directory: Option<&std::path::Path>) -> Model {
    let mut model = Model::new(Instant::now());
    model.blocks.clear();
    let directory = directory.map(|path| {
        let path = path.to_string_lossy();
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    });
    model.account = Some(View {
        phase: Phase::Login,
        notice: "Connecting for sign-in / Esc cancels".into(),
        local_notice: String::new(),
        local_failed: false,
        failed: false,
        partial_output: false,
        turns: 0,
        pending: None,
        recovery: None,
        active_tool: None,
        edit: None,
        command: None,
        selected,
        catalog: None,
        pending_catalog: None,
        id: None,
        directory,
        file_tools: false,
        access: crate::workspace::Access::Workspace,
        local_operation: None,
    });
    model
}

pub(super) fn input(model: &mut Model, key: Key, session: &mut Session) {
    attach_queue(model, session);
    if super::commands::input(model, &key, session) {
        return;
    }
    match &key {
        Key::RetrieveQueued => {
            super::recovery::retrieve(model, session);
            return;
        }
        Key::AbandonRecovered => {
            super::recovery::abandon(model);
            return;
        }
        _ => {}
    }
    let Some(view) = &mut model.account else {
        return;
    };
    match key {
        Key::Enter => {
            if view.recovery.is_some() && model.prompt_history.browsing() {
                view.local_notice =
                    "Return to the recovered edit with Ctrl+N before sending / drafts kept".into();
                view.local_failed = false;
                return;
            }
            if model.editor.text.len() > session::MAX_PROMPT_BYTES {
                view.local_notice = "Prompt exceeds 8 KiB / draft kept".into();
                view.local_failed = true;
                return;
            }
            if view.phase == Phase::SignedOut && !model.editor.text.trim().is_empty() {
                view.notice = "Signed out / use /login to sign in; draft kept and will not send automatically".into();
                return;
            }
            if view.phase == Phase::Generating && !model.editor.text.trim().is_empty() {
                if session.enqueue(&model.editor.text) {
                    let prompt = model.editor.take();
                    view.local_notice.clear();
                    view.local_failed = false;
                    super::recovery::submitted(model, &prompt, false);
                } else {
                    view.local_notice = "Queue full or stopping / draft kept".into();
                    view.local_failed = false;
                }
                return;
            }
            if view.phase != Phase::Ready || model.editor.text.trim().is_empty() {
                return;
            }
            if view.turns >= 256 {
                view.local_notice =
                    "Session limit reached / draft kept; restart to continue".into();
                view.local_failed = true;
                return;
            }
            if !session.submit(&model.editor.text) {
                view.local_notice = if session.selection_unavailable() {
                    UNAVAILABLE_SELECTION_NOTICE.into()
                } else {
                    "Message not sent / draft kept".into()
                };
                view.local_failed = true;
                return;
            }
            view.turns += 1;
            let prompt = model.editor.take();
            view.phase = Phase::Generating;
            view.failed = false;
            view.partial_output = false;
            view.local_notice.clear();
            view.local_failed = false;
            view.notice = "Waiting for model".into();
            model.status_spinner.reset(Instant::now());
            super::recovery::submitted(model, &prompt, true);
            model.blocks.push(Block {
                speaker: "You",
                text: prompt,
            });
            model.blocks.push(Block {
                speaker: "Assistant",
                text: String::new(),
            });
        }
        Key::Escape | Key::Interrupt
            if matches!(view.phase, Phase::Login | Phase::Generating)
                || view.pending_catalog.is_some() =>
        {
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

pub(super) fn attach_queue(model: &mut Model, session: &Session) {
    if let Some(view) = &mut model.account
        && view.pending.is_none()
    {
        view.pending = Some(session.pending_guidance());
    }
}

pub(super) fn event(model: &mut Model, event: Event) {
    if matches!(
        event,
        Event::Finished(..) | Event::LoginFailed(_) | Event::LoggedOut
    ) {
        super::edit_view::stop(model);
        super::command_view::stop(model);
    }
    let Some(view) = &mut model.account else {
        return;
    };
    match event {
        Event::Guidance { text, new_turn } => {
            if new_turn {
                view.turns += 1;
                model.prompt_history.record_new_turn(&text);
            }
            view.phase = Phase::Generating;
            view.partial_output = false;
            view.notice = "Waiting for model".into();
            model.status_spinner.reset(Instant::now());
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
        Event::EditPlanned { id, preview } if view.phase == Phase::Generating => {
            super::edit_view::planned(model, id, preview)
        }
        Event::EditFinished {
            id,
            summary,
            applied,
            failed,
        } => super::edit_view::finished(model, id, summary, applied, failed),
        Event::CommandPlanned { id, preview } if view.phase == Phase::Generating => {
            super::command_view::planned(model, id, preview)
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
            view.failed = false;
            view.notice.clear();
        }
        Event::CatalogLoaded(catalog) => {
            let unavailable = catalog.support(view.selected)
                == crate::providers::openai_account::catalog::Support::Unsupported;
            view.catalog = Some(catalog.clone());
            if unavailable {
                view.local_notice =
                    "Selected model or effort unavailable in this account catalog / use /model"
                        .into();
                view.local_failed = true;
            }
            if let Some(defaults) = view.pending_catalog.take() {
                let current = if defaults {
                    crate::state::settings::Settings::user()
                        .map(|settings| settings.model)
                        .unwrap_or(view.selected)
                } else {
                    view.selected
                };
                model
                    .menu
                    .open(super::menu::models(&catalog, current, defaults));
            }
        }
        Event::CatalogFailed(kind) => {
            view.catalog = None;
            view.pending_catalog = None;
            view.local_notice = match kind {
                session::CatalogFailure::Unavailable => {
                    "Account model catalog unavailable / selection kept; /model retries"
                }
                session::CatalogFailure::Invalid => {
                    "Account model catalog malformed or oversized / selection kept; /model retries"
                }
                session::CatalogFailure::Empty => {
                    "Account model catalog has no usable choices / selection kept; /model retries"
                }
                session::CatalogFailure::Cancelled => {
                    "Account model loading cancelled / selection kept; /model retries"
                }
            }
            .into();
            view.local_failed = true;
        }
        Event::ModelChanged(selected) => {
            view.selected = selected;
            view.phase = Phase::Ready;
            view.local_operation = None;
            view.notice.clear();
        }
        Event::Thinking if view.phase == Phase::Generating => {
            view.notice = "Thinking".into();
            model.tools.waiting("Thinking");
        }
        Event::RequestStarted if view.phase == Phase::Generating => {
            view.partial_output = false;
            model.status_spinner.reset(Instant::now());
            model.blocks.push(Block {
                speaker: "Assistant",
                text: String::new(),
            });
            view.notice = "Waiting for model".into();
            model.tools.waiting("Waiting for model");
        }
        Event::Retrying if view.phase == Phase::Generating => {
            view.notice = "Connection failed before request submission / retrying".into();
            model.tools.waiting("Retrying connection");
        }
        Event::ToolStarted { name, path } if view.phase == Phase::Generating => {
            view.notice = "Reading workspace".into();
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
                view.partial_output = true;
            }
            if let Some(block) = model.blocks.last_mut()
                && block.speaker == "Assistant"
            {
                block.text.push_str(&text);
            }
            view.notice = "Streaming".into();
        }
        Event::TextReconciled(text) if view.phase == Phase::Generating => {
            if let Some(block) = model.blocks.last()
                && block.speaker == "Assistant"
            {
                // Emitted rows may already be in immutable terminal scrollback.
                // Keep that visible preview and append the corrected passage
                // or the complete validated answer as one Markdown block.
                model.blocks.push(Block {
                    speaker: "Correction",
                    text: super::reconcile::display(&block.text, &text),
                });
            }
        }
        Event::Finished(end, metrics) => {
            let signing_out = view.phase == Phase::SigningOut;
            model.tools.close(
                &model.blocks,
                !matches!(end, End::Complete | End::Refused),
                Instant::now(),
            );
            view.local_notice.clear();
            view.local_failed = false;
            view.phase = if end == End::Failed(session::Failure::Storage) {
                Phase::Closed
            } else if end.needs_login() {
                Phase::SignedOut
            } else {
                Phase::Ready
            };
            view.failed = matches!(end, End::Failed(_));
            view.notice = completion(end, metrics, view.partial_output);
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
            } else if view.phase == Phase::SignedOut {
                view.notice = "Account access ended / use /login to sign in; draft kept".into();
            } else if signing_out {
                view.phase = Phase::SigningOut;
                view.notice = "Signing out / waiting for cleanup".into();
            }
        }
        Event::LoginFailed(failure) => {
            view.catalog = None;
            view.pending_catalog = None;
            model.tools.close(&model.blocks, true, Instant::now());
            let signing_out =
                view.phase == Phase::SigningOut && failure != session::Failure::Worker;
            view.phase = if failure == session::Failure::Worker {
                Phase::Closed
            } else if signing_out {
                Phase::SigningOut
            } else {
                Phase::SignedOut
            };
            view.failed = failure != session::Failure::Cancelled;
            view.notice = if signing_out {
                "Signing out / waiting for cleanup".into()
            } else if failure == session::Failure::Cancelled {
                "Sign-in cancelled / use /login to retry; session and draft kept".into()
            } else if failure == session::Failure::Worker {
                format!("{failure}\nRestart Jecode to recover / Ctrl+Q exits")
            } else {
                format!("{failure}\nUse /login to retry; your conversation and draft are kept")
            };
        }
        Event::LoggedOut => {
            view.catalog = None;
            view.pending_catalog = None;
            model.menu.close();
            model.tools.close(&model.blocks, true, Instant::now());
            view.phase = Phase::SignedOut;
            view.failed = false;
            view.notice =
                "Signed out locally / use /login to sign in; session and draft kept".into();
        }
        Event::LogoutFailed(failure, was_signed_in) => {
            view.phase = if was_signed_in {
                Phase::Ready
            } else {
                Phase::SignedOut
            };
            view.failed = true;
            view.notice = format!(
                "Could not remove saved account: {failure}\nUse /logout to retry; session and draft kept"
            );
        }
        Event::Thinking
        | Event::EditPlanned { .. }
        | Event::CommandPlanned { .. }
        | Event::Text(_)
        | Event::TextReconciled(_)
        | Event::RequestStarted
        | Event::Retrying
        | Event::ToolStarted { .. }
        | Event::ToolFinished { .. } => {}
    }
}
pub(super) fn compacting(model: &mut Model) {
    if let Some(view) = &mut model.account {
        view.phase = Phase::Generating;
        view.local_operation = Some(LocalOperation::Compact(String::new()));
        view.notice = "Compacting context".into();
        model.status_spinner.reset(Instant::now());
    }
}
pub(super) fn discarding_pending_images(model: &mut Model) {
    if let Some(view) = &mut model.account {
        view.phase = Phase::Generating;
        view.local_operation = Some(LocalOperation::Compact(String::new()));
        view.notice = "Discarding pending visual input".into();
    }
}
fn completion(end: End, metrics: Metrics, partial_output: bool) -> String {
    let mut result = match end {
        End::Complete => "Complete".into(),
        End::Refused => "Response refused".into(),
        End::Incomplete => "Incomplete response / partial output retained".into(),
        End::Failed(failure) => failure.to_string(),
    };
    if partial_output && matches!(end, End::Failed(session::Failure::Account(_))) {
        result.push_str(" / partial output retained");
    }
    result.push_str(&format!(" / {:.1}s", metrics.elapsed_ms as f64 / 1000.0));
    if metrics.tool_calls != 0 {
        result.push_str(&format!(
            " / {} tools / {} requests",
            metrics.tool_calls, metrics.requests
        ));
    }
    if metrics.connection_attempts != 0 {
        result.push_str(&format!(
            " / {} connections / {} submitted",
            metrics.connection_attempts, metrics.submissions
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
