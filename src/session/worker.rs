use super::{CatalogFailure, Command, End, Event, Failure, Metrics, Model, history::History};
use crate::{
    providers::openai_account::{Progress, Request, Response, client},
    tls::{Budget, NetworkError},
};
use std::{
    ops::ControlFlow,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

pub(crate) trait Backend: Send {
    fn login(
        &mut self,
        budget: &Budget<'_>,
        code: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error>;
    fn logout(&mut self, _: &Budget<'_>) -> Result<(), client::Error> {
        Ok(())
    }
    fn catalog(
        &mut self,
        _: &Budget<'_>,
    ) -> Result<Option<crate::providers::openai_account::catalog::Catalog>, client::Error> {
        Ok(None)
    }
    fn generate(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error>;
}
pub(super) struct Account {
    client: Option<client::Client>,
    idle_timeout: Duration,
}
impl Default for Account {
    fn default() -> Self {
        Self::with_idle_timeout(client::DEFAULT_STREAM_IDLE_TIMEOUT_MS)
    }
}
impl Account {
    pub(super) fn with_idle_timeout(milliseconds: u64) -> Self {
        Self {
            client: None,
            idle_timeout: Duration::from_millis(milliseconds),
        }
    }
}
#[cfg(test)]
pub(crate) type EffectGate = Arc<dyn Fn(&str, &SyncSender<Event>) + Send + Sync>;
impl Backend for Account {
    fn login(
        &mut self,
        budget: &Budget<'_>,
        code: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        self.client = Some(client::Client::connect(budget, code)?);
        Ok(())
    }
    fn logout(&mut self, budget: &Budget<'_>) -> Result<(), client::Error> {
        client::Client::logout(budget)?;
        self.client = None;
        Ok(())
    }
    fn catalog(
        &mut self,
        budget: &Budget<'_>,
    ) -> Result<Option<crate::providers::openai_account::catalog::Catalog>, client::Error> {
        self.client
            .as_mut()
            .ok_or(client::Error::Expired)?
            .catalog(budget)
            .map(Some)
    }
    fn generate(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.client
            .as_mut()
            .ok_or(client::Error::Expired)?
            .generate_with_idle(request, budget, self.idle_timeout, progress)
    }
}

pub(super) struct Context {
    pub events: SyncSender<Event>,
    pub cancelled: Arc<AtomicBool>,
    pub stopped: Arc<AtomicBool>,
    pub guidance: Arc<super::queue::Pending>,
    pub next_effect: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    pub effect_gate: Option<EffectGate>,
}
impl Context {
    pub(super) fn send(&self, event: Event, cancellable: bool) -> ControlFlow<()> {
        self.deliver(event, cancellable)
    }
    /// Transient presentation must not stall a completed effect or a live process.
    pub(super) fn notify(&self, event: Event) -> bool {
        !self.stopped.load(Ordering::Acquire) && self.events.try_send(event).is_ok()
    }
    fn deliver(&self, mut event: Event, cancellable: bool) -> ControlFlow<()> {
        loop {
            if self.stopped.load(Ordering::Acquire)
                || cancellable && self.cancelled.load(Ordering::Acquire)
            {
                return ControlFlow::Break(());
            }
            match self.events.try_send(event) {
                Ok(()) => return ControlFlow::Continue(()),
                Err(TrySendError::Disconnected(_)) => return ControlFlow::Break(()),
                Err(TrySendError::Full(returned)) => event = returned,
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
    pub(super) fn text(&self, mut text: &str, cancellable: bool) -> ControlFlow<()> {
        while !text.is_empty() {
            let mut end = text.len().min(4096);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.send(Event::Text(text[..end].into()), cancellable)?;
            text = &text[end..];
        }
        ControlFlow::Continue(())
    }
    pub(super) fn check(&self) -> Result<(), Failure> {
        if self.cancelled.load(Ordering::Acquire) || self.stopped.load(Ordering::Acquire) {
            Err(Failure::Cancelled)
        } else {
            Ok(())
        }
    }
    #[cfg(test)]
    pub(super) fn before_effect(&self, name: &str) {
        if let Some(gate) = &self.effect_gate {
            gate(name, &self.events);
        }
    }
}

pub(super) fn run(
    mut model: Model,
    mut backend: impl Backend,
    commands: Receiver<Command>,
    context: Context,
    workspace: Option<crate::workspace::Workspace>,
    mut history: History,
) {
    if let Some(record) = &history.record
        && context
            .send(
                Event::Restored {
                    id: record.id().into(),
                    items: history.transcript(),
                    turns: history.turn_count(),
                },
                false,
            )
            .is_break()
    {
        return;
    }
    let mut catalog = None;
    let mut signed_in = login(&mut backend, &context, &mut history, model, &mut catalog);
    loop {
        let command = match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(command) => command,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let Some(text) = context.guidance.claim() else {
                    continue;
                };
                if context.cancelled.load(Ordering::Acquire) {
                    let _ = context.send(Event::GuidanceReturned(text), false);
                    continue;
                }
                if context
                    .send(
                        Event::Guidance {
                            text: text.clone(),
                            new_turn: true,
                        },
                        false,
                    )
                    .is_break()
                {
                    break;
                }
                Command::Prompt(text)
            }
        };
        if context.stopped.load(Ordering::Acquire) {
            break;
        }
        match command {
            Command::Login => {
                if signed_in {
                    let _ = context.send(Event::Ready, false);
                } else {
                    signed_in = login(&mut backend, &context, &mut history, model, &mut catalog);
                }
                continue;
            }
            Command::Catalog => {
                signed_in = load_catalog(&mut backend, &context, &mut history, model, &mut catalog);
                if signed_in {
                    let _ = context.send(Event::Ready, false);
                }
                continue;
            }
            Command::Logout => {
                // Guidance accepted during the previous turn must be returned
                // before sign-out completes, so a later login cannot send it.
                super::queue::return_pending(&context);
                let unaffected = AtomicBool::new(false);
                let budget = Budget {
                    cancelled: &unaffected,
                    deadline: Some(Instant::now() + Duration::from_secs(5)),
                };
                match backend.logout(&budget) {
                    Ok(()) => {
                        signed_in = false;
                        catalog = None;
                        history.image_capable = false;
                        let _ = context.send(Event::LoggedOut, false);
                    }
                    Err(error) => {
                        let _ = context.send(
                            Event::LogoutFailed(Failure::Account(error), signed_in),
                            false,
                        );
                    }
                }
                continue;
            }
            _ => {}
        }
        let started = Instant::now();
        let mut metrics = Metrics::default();
        let prompt = match command {
            Command::Login | Command::Logout | Command::Catalog => unreachable!(),
            Command::Prompt(prompt) => prompt,
            Command::Model(selected) => {
                if history.set_model(selected).is_err() {
                    let _ = context.send(
                        Event::Finished(End::Failed(Failure::Storage), metrics),
                        false,
                    );
                    break;
                }
                model = selected;
                history.image_capable = catalog.as_ref().is_some_and(
                    |catalog: &crate::providers::openai_account::catalog::Catalog| {
                        catalog.image_support(model)
                            == crate::providers::openai_account::catalog::Support::Supported
                    },
                );
                if context.send(Event::ModelChanged(model), false).is_break() {
                    break;
                }
                if !history.image_capable && history.has_retained_images() {
                    let _ = context.send(Event::ContextReport("This model has no verified image input. Saved image evidence remains intact, but its pixels are not visible until an image-capable model is selected.".into()), false);
                }
                continue;
            }
            Command::Inspect => {
                let _ = context.send(
                    Event::ContextReport(super::context::report(
                        &history,
                        model,
                        workspace.is_some(),
                    )),
                    false,
                );
                continue;
            }
            Command::Compact => {
                let result = super::context::compact(
                    &mut backend,
                    &mut history,
                    &context,
                    model,
                    workspace.is_some(),
                    &mut metrics,
                );
                metrics.elapsed_ms = millis(started);
                let end = result.map_or_else(End::Failed, |()| End::Complete);
                if end.needs_login() {
                    signed_in = false;
                }
                let _ = context.send(Event::Finished(end, metrics), false);
                if end != End::Complete {
                    super::queue::return_pending(&context);
                }
                if end == End::Failed(Failure::Storage) {
                    break;
                }
                continue;
            }
            Command::DiscardPendingImages => {
                let result = history.abandon_pending_images();
                if let Ok(changed) = result {
                    let message = if changed {
                        "Pending visual input discarded without inspection. Saved receipts and exact image bytes remain; request smaller views or use a saved image_id to inspect again."
                    } else {
                        "No pending visual input to discard. Saved image evidence remains intact."
                    };
                    let _ = context.send(Event::ContextReport(message.into()), false);
                }
                let end = result.map_or_else(End::Failed, |_| End::Complete);
                let _ = context.send(Event::Finished(end, metrics), false);
                if end == End::Failed(Failure::Storage) {
                    break;
                }
                continue;
            }
        };
        if catalog.as_ref().is_some_and(
            |catalog: &crate::providers::openai_account::catalog::Catalog| !catalog.fresh(),
        ) {
            let _ = load_catalog(&mut backend, &context, &mut history, model, &mut catalog);
        }
        // Compaction may release earlier resident turns while this one runs.
        let turn_number = history.turn_count();
        let result = history
            .begin(prompt)
            .and_then(|()| history.checkpoint())
            .and_then(|()| {
                super::tool_loop::run(
                    &mut backend,
                    &mut history,
                    &context,
                    model,
                    workspace.as_ref(),
                    started,
                    Instant::now,
                    &mut metrics,
                )
            });
        let mut end = result.unwrap_or_else(End::Failed);
        metrics.elapsed_ms = millis(started);
        let current = if history.turn_count() == turn_number + 1 {
            turn_number
                .checked_sub(history.base_turn)
                .and_then(|index| history.turns.get_mut(index))
        } else {
            None
        };
        if let Some(turn) = current {
            turn.end = Some(end);
            turn.outcome = match end {
                End::Complete => "Complete".into(),
                End::Incomplete => "Incomplete response".into(),
                End::Refused => "Response refused".into(),
                End::Failed(f) => f.to_string(),
            };
            turn.metrics = metrics;
        } else if history.turn_count() != turn_number {
            end = End::Failed(Failure::Storage);
        }
        if history.checkpoint().is_err() {
            end = End::Failed(Failure::Storage);
        }
        if end != End::Complete {
            super::queue::return_pending(&context);
        }
        if end.needs_login() {
            signed_in = false;
        }
        if context
            .send(Event::Finished(end, metrics), false)
            .is_break()
        {
            break;
        }
        if end == End::Failed(Failure::Storage) {
            break;
        }
    }
}

fn login(
    backend: &mut impl Backend,
    context: &Context,
    history: &mut History,
    model: Model,
    catalog: &mut Option<crate::providers::openai_account::catalog::Catalog>,
) -> bool {
    let result = backend.login(
        &Budget {
            deadline: Some(Instant::now() + Duration::from_secs(900)),
            cancelled: &context.cancelled,
        },
        &mut |code| context.send(Event::LoginCode(code.into()), true),
    );
    match result {
        Ok(()) => {
            if load_catalog(backend, context, history, model, catalog) {
                context.send(Event::Ready, false).is_continue()
            } else {
                false
            }
        }
        Err(error) => {
            let _ = context.send(Event::LoginFailed(failure(error, context)), false);
            false
        }
    }
}

fn load_catalog(
    backend: &mut impl Backend,
    context: &Context,
    history: &mut History,
    model: Model,
    current: &mut Option<crate::providers::openai_account::catalog::Catalog>,
) -> bool {
    let budget = Budget {
        deadline: Some(Instant::now() + Duration::from_secs(5)),
        cancelled: &context.cancelled,
    };
    match backend.catalog(&budget) {
        Ok(Some(catalog)) => {
            history.image_capable = catalog.image_support(model)
                == crate::providers::openai_account::catalog::Support::Supported;
            *current = Some(catalog.clone());
            let _ = context.send(Event::CatalogLoaded(catalog), false);
        }
        Ok(None) => {
            history.image_capable = false;
            *current = None;
        }
        Err(error)
            if matches!(
                error,
                client::Error::Expired
                    | client::Error::AccountChanged
                    | client::Error::Login(crate::providers::openai_account::auth::Error::Denied)
            ) =>
        {
            let _ = context.send(Event::LoginFailed(Failure::Account(error)), false);
            return false;
        }
        Err(error) => {
            history.image_capable = false;
            *current = None;
            let kind = match error {
                client::Error::Catalog(crate::providers::openai_account::catalog::Error::Empty) => {
                    CatalogFailure::Empty
                }
                client::Error::Catalog(_) => CatalogFailure::Invalid,
                _ if context.cancelled.load(Ordering::Acquire) => CatalogFailure::Cancelled,
                _ => CatalogFailure::Unavailable,
            };
            let _ = context.send(Event::CatalogFailed(kind), false);
        }
    }
    true
}

pub(super) fn failure(error: client::Error, context: &Context) -> Failure {
    if context.cancelled.load(Ordering::Acquire)
        || context.stopped.load(Ordering::Acquire)
        || error == client::Error::Network(NetworkError::Cancelled)
        || matches!(
            error,
            client::Error::Response {
                error: crate::providers::openai_account::Error::Cancelled,
                ..
            }
        )
        || matches!(
            error,
            client::Error::Transport {
                error: NetworkError::Cancelled,
                ..
            }
        )
    {
        Failure::Cancelled
    } else {
        Failure::Account(error)
    }
}
pub(super) fn millis(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Start an I/O deadline at the operation boundary, independently of task age.
pub(super) fn operation_deadline(now: Instant, timeout: Duration) -> Instant {
    now + timeout
}
