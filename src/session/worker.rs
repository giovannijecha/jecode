use super::{Command, End, Event, Failure, Metrics, Model, history::History};
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
    fn generate(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error>;
}
#[derive(Default)]
pub(super) struct Account(Option<client::Client>);
impl Backend for Account {
    fn login(
        &mut self,
        budget: &Budget<'_>,
        code: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        self.0 = Some(client::Client::connect(budget, code)?);
        Ok(())
    }
    fn generate(
        &mut self,
        request: &Request,
        budget: &Budget<'_>,
        progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        self.0
            .as_mut()
            .ok_or(client::Error::Expired)?
            .generate(request, budget, progress)
    }
}

pub(super) struct Context {
    pub events: SyncSender<Event>,
    pub cancelled: Arc<AtomicBool>,
    pub stopped: Arc<AtomicBool>,
    pub decisions: Receiver<super::approval::Decision>,
    pub guidance: Receiver<String>,
    pub next_approval: std::sync::atomic::AtomicU64,
}
impl Context {
    pub(super) fn send(&self, event: Event, cancellable: bool) -> ControlFlow<()> {
        self.deliver(event, cancellable, None)
    }
    pub(super) fn send_until(&self, event: Event, deadline: Instant) -> ControlFlow<()> {
        self.deliver(event, true, Some(deadline))
    }
    fn deliver(
        &self,
        mut event: Event,
        cancellable: bool,
        deadline: Option<Instant>,
    ) -> ControlFlow<()> {
        loop {
            if self.stopped.load(Ordering::Acquire)
                || cancellable && self.cancelled.load(Ordering::Acquire)
                || deadline.is_some_and(|deadline| Instant::now() >= deadline)
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
}

pub(super) fn run(
    mut model: Model,
    mut backend: impl Backend,
    commands: Receiver<Command>,
    context: Context,
    workspace: Option<crate::workspace::Workspace>,
    mut history: History,
) {
    let login = backend.login(
        &Budget {
            deadline: Instant::now() + Duration::from_secs(900),
            cancelled: &context.cancelled,
        },
        &mut |code| context.send(Event::LoginCode(code.into()), true),
    );
    if let Err(error) = login {
        let _ = context.send(Event::LoginFailed(failure(error, &context)), false);
        return;
    }
    if let Some(record) = &history.record
        && context
            .send(
                Event::Restored {
                    id: record.id().into(),
                    items: history.transcript(),
                    turns: history.turns.len(),
                },
                false,
            )
            .is_break()
    {
        return;
    }
    if context.send(Event::Ready, false).is_break() {
        return;
    }
    loop {
        let command = match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(command) => command,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let Ok(text) = context.guidance.try_recv() else {
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
        let started = Instant::now();
        let mut metrics = Metrics::default();
        let prompt = match command {
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
                if context.send(Event::ModelChanged(model), false).is_break() {
                    break;
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
                    &mut metrics,
                );
                metrics.elapsed_ms = millis(started);
                let end = result.map_or_else(End::Failed, |()| End::Complete);
                let _ = context.send(Event::Finished(end, metrics), false);
                if end != End::Complete {
                    super::queue::return_pending(&context);
                }
                if end == End::Failed(Failure::Storage) {
                    break;
                }
                continue;
            }
        };
        let index = history.turns.len();
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
                    &mut metrics,
                )
            });
        let mut end = result.unwrap_or_else(End::Failed);
        metrics.elapsed_ms = millis(started);
        if let Some(turn) = history.turns.get_mut(index) {
            turn.end = Some(end);
            turn.outcome = match end {
                End::Complete => "Complete".into(),
                End::Incomplete => "Incomplete response".into(),
                End::Refused => "Response refused".into(),
                End::Failed(f) => f.to_string(),
            };
            turn.metrics = metrics;
        }
        if history.checkpoint().is_err() {
            end = End::Failed(Failure::Storage);
        }
        if end != End::Complete {
            super::queue::return_pending(&context);
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

pub(super) fn failure(error: client::Error, context: &Context) -> Failure {
    if context.cancelled.load(Ordering::Acquire)
        || error == client::Error::Network(NetworkError::Cancelled)
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
