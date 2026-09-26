//! One generation request over a verified single-use connection.
use super::{
    Delivery, Error, RequestStage,
    recovery::{Termination, Trace},
    timing::{self, ResponseWindow},
};
use crate::{
    providers::openai_account::{HttpResponseStream, Limits, Progress, Response},
    tls::{ApplicationWrite, Budget, Connection, NetworkError, Plaintext},
};
use std::{
    ops::ControlFlow,
    time::{Duration, Instant},
};

pub(super) trait ResponseChannel {
    fn write(
        &mut self,
        bytes: &[u8],
        budget: &Budget<'_>,
        progress: &mut ApplicationWrite,
    ) -> Result<(), NetworkError>;
    fn read(&mut self, budget: &Budget<'_>) -> Result<Option<Plaintext>, NetworkError>;
    fn read_observed(
        &mut self,
        budget: &Budget<'_>,
        _: &mut usize,
    ) -> Result<Option<Plaintext>, NetworkError> {
        self.read(budget)
    }
    fn close(&mut self, budget: &Budget<'_>) -> Result<(), NetworkError>;
}
impl ResponseChannel for Connection {
    fn write(
        &mut self,
        bytes: &[u8],
        budget: &Budget<'_>,
        progress: &mut ApplicationWrite,
    ) -> Result<(), NetworkError> {
        Connection::write_observed(self, bytes, budget, progress)
    }
    fn read(&mut self, budget: &Budget<'_>) -> Result<Option<Plaintext>, NetworkError> {
        Connection::read(self, budget)
    }
    fn read_observed(
        &mut self,
        budget: &Budget<'_>,
        received_wire_bytes: &mut usize,
    ) -> Result<Option<Plaintext>, NetworkError> {
        Connection::read_observed(self, budget, received_wire_bytes)
    }
    fn close(&mut self, budget: &Budget<'_>) -> Result<(), NetworkError> {
        Connection::close(self, budget)
    }
}

pub(super) struct ExchangeProgress<'a> {
    pub delivery: &'a mut Delivery,
    pub write: &'a mut ApplicationWrite,
    pub trace: &'a mut Trace,
}
pub(super) struct RequestWindows<'a, 'b> {
    pub write: &'a Budget<'b>,
    pub idle: Duration,
}

#[cfg(test)]
pub(super) fn exchange(
    connection: &mut impl ResponseChannel,
    bytes: &[u8],
    budget: &Budget<'_>,
    write_budget: &Budget<'_>,
    state: ExchangeProgress<'_>,
    progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
) -> Result<Response, Error> {
    exchange_with_idle(
        connection,
        bytes,
        budget,
        write_budget,
        Duration::from_millis(timing::DEFAULT_STREAM_IDLE_TIMEOUT_MS),
        state,
        progress,
    )
}

pub(super) fn exchange_with_idle(
    connection: &mut impl ResponseChannel,
    bytes: &[u8],
    budget: &Budget<'_>,
    write_budget: &Budget<'_>,
    idle_timeout: Duration,
    state: ExchangeProgress<'_>,
    progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
) -> Result<Response, Error> {
    exchange_with_clock(
        connection,
        bytes,
        budget,
        RequestWindows {
            write: write_budget,
            idle: idle_timeout,
        },
        state,
        progress,
        Instant::now,
    )
}

pub(super) fn exchange_with_clock(
    connection: &mut impl ResponseChannel,
    bytes: &[u8],
    budget: &Budget<'_>,
    windows: RequestWindows<'_, '_>,
    state: ExchangeProgress<'_>,
    progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
    now: impl Fn() -> Instant,
) -> Result<Response, Error> {
    let ExchangeProgress {
        delivery,
        write,
        trace,
    } = state;
    let stage_started = now();
    let request_started = *trace.started.get_or_insert(stage_started);
    connection
        .write(bytes, windows.write, write)
        .map_err(|error| {
            trace.stage_elapsed_ms = elapsed_ms(stage_started, now());
            trace.request_elapsed_ms = elapsed_ms(request_started, now());
            trace.termination =
                timing::stage_termination(error, budget, now(), Termination::WriteTimeout);
            *delivery = if write.accepted_wire_bytes == 0 {
                Delivery::NotSubmitted
            } else {
                Delivery::PossiblySubmitted
            };
            Error::Transport {
                stage: RequestStage::RequestWrite,
                error,
                delivery: *delivery,
                accepted_wire_bytes: write.accepted_wire_bytes,
            }
        })?;
    *delivery = Delivery::PossiblySubmitted;
    let mut response = HttpResponseStream::new(Limits {
        event_bytes: 1024 * 1024,
        output_bytes: 1024 * 1024,
        ..Limits::default()
    });
    let mut progress = progress;
    let stage_started = now();
    let mut window = ResponseWindow::new(stage_started, windows.idle);
    while !response.is_finished() {
        let before_read = now();
        if let Err(kind) = window.check(budget, before_read) {
            record_read_failure(
                trace,
                request_started,
                stage_started,
                &window,
                before_read,
                kind,
            );
            return Err(read_termination(kind, *delivery, write.accepted_wire_bytes));
        }
        let read_budget = Budget {
            deadline: Some(window.deadline(budget)),
            cancelled: budget.cancelled,
        };
        let Some(bytes) = connection
            .read_observed(&read_budget, &mut trace.received_wire_bytes)
            .map_err(|error| {
                let at = now();
                let kind = timing::stage_termination(error, budget, at, window.inactivity_kind());
                if let Some(kind) = kind {
                    record_read_failure(trace, request_started, stage_started, &window, at, kind);
                } else {
                    trace.stage_elapsed_ms = elapsed_ms(stage_started, at);
                    trace.request_elapsed_ms = elapsed_ms(request_started, at);
                    trace.since_progress_ms = Some(window.since_progress(at));
                }
                Error::Transport {
                    stage: RequestStage::ResponseRead,
                    error,
                    delivery: *delivery,
                    accepted_wire_bytes: write.accepted_wire_bytes,
                }
            })?
        else {
            break;
        };
        let received_at = now();
        if let Err(kind) = window.check(budget, received_at) {
            record_read_failure(
                trace,
                request_started,
                stage_started,
                &window,
                received_at,
                kind,
            );
            return Err(read_termination(kind, *delivery, write.accepted_wire_bytes));
        }
        trace.response_plaintext_bytes = trace
            .response_plaintext_bytes
            .saturating_add(bytes.bytes.len());
        let before_events = response.stream_events();
        response
            .push(&bytes.bytes, &mut progress)
            .map_err(|error| {
                trace.stage_elapsed_ms = elapsed_ms(stage_started, now());
                trace.request_elapsed_ms = elapsed_ms(request_started, now());
                trace.since_progress_ms = Some(window.since_progress(now()));
                trace.response_status = response.response_status();
                trace.stream_events = response.stream_events();
                if response.stream_started() {
                    *delivery = Delivery::Streaming;
                }
                Error::Response {
                    error,
                    delivery: *delivery,
                }
            })?;
        // A slow local progress callback must not extend provider inactivity.
        window.observed(before_events, response.stream_events(), received_at);
        trace.response_status = response.response_status();
        trace.stream_events = response.stream_events();
        if response.stream_started() {
            *delivery = Delivery::Streaming;
        }
    }
    let result = response.finish().map_err(|error| {
        trace.stage_elapsed_ms = elapsed_ms(stage_started, now());
        trace.request_elapsed_ms = elapsed_ms(request_started, now());
        trace.since_progress_ms = Some(window.since_progress(now()));
        Error::Response {
            error,
            delivery: *delivery,
        }
    })?;
    *delivery = Delivery::Completed;
    trace.request_elapsed_ms = elapsed_ms(request_started, now());
    // A validated model completion survives best-effort close failure.
    let _ = connection.close(&Budget {
        deadline: Some(timing::stage_deadline(
            budget,
            Instant::now(),
            Duration::from_millis(100),
        )),
        cancelled: budget.cancelled,
    });
    Ok(result)
}

fn elapsed_ms(start: Instant, now: Instant) -> u64 {
    now.saturating_duration_since(start)
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
fn record_read_failure(
    trace: &mut Trace,
    request_start: Instant,
    response_start: Instant,
    window: &ResponseWindow,
    now: Instant,
    kind: Termination,
) {
    trace.stage_elapsed_ms = elapsed_ms(response_start, now);
    trace.request_elapsed_ms = elapsed_ms(request_start, now);
    trace.since_progress_ms = Some(window.since_progress(now));
    trace.termination = Some(kind);
}
fn read_termination(kind: Termination, delivery: Delivery, accepted_wire_bytes: usize) -> Error {
    Error::Transport {
        stage: RequestStage::ResponseRead,
        error: if kind == Termination::Cancelled {
            NetworkError::Cancelled
        } else {
            NetworkError::Timeout
        },
        delivery,
        accepted_wire_bytes,
    }
}
