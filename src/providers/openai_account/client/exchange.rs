//! One generation request over a verified single-use connection.
use super::{Delivery, Error, RequestStage, recovery::Trace};
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

pub(super) fn exchange(
    connection: &mut impl ResponseChannel,
    bytes: &[u8],
    budget: &Budget<'_>,
    write_budget: &Budget<'_>,
    state: ExchangeProgress<'_>,
    progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
) -> Result<Response, Error> {
    let ExchangeProgress {
        delivery,
        write,
        trace,
    } = state;
    let stage_started = Instant::now();
    connection
        .write(bytes, write_budget, write)
        .map_err(|error| {
            trace.stage_elapsed_ms = stage_started.elapsed().as_millis() as u64;
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
    let stage_started = Instant::now();
    while !response.is_finished() {
        let Some(bytes) = connection
            .read_observed(budget, &mut trace.received_wire_bytes)
            .map_err(|error| {
                trace.stage_elapsed_ms = stage_started.elapsed().as_millis() as u64;
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
        trace.response_plaintext_bytes = trace
            .response_plaintext_bytes
            .saturating_add(bytes.bytes.len());
        response
            .push(&bytes.bytes, &mut progress)
            .map_err(|error| {
                trace.stage_elapsed_ms = stage_started.elapsed().as_millis() as u64;
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
        trace.response_status = response.response_status();
        trace.stream_events = response.stream_events();
        if response.stream_started() {
            *delivery = Delivery::Streaming;
        }
    }
    let result = response.finish().map_err(|error| {
        trace.stage_elapsed_ms = stage_started.elapsed().as_millis() as u64;
        Error::Response {
            error,
            delivery: *delivery,
        }
    })?;
    *delivery = Delivery::Completed;
    // A validated model completion survives best-effort close failure.
    let _ = connection.close(&Budget {
        deadline: budget
            .deadline
            .min(Instant::now() + Duration::from_millis(100)),
        cancelled: budget.cancelled,
    });
    Ok(result)
}
