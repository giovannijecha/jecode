//! One generation request over a verified single-use connection.
use super::{Delivery, Error, RequestStage};
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
    fn close(&mut self, budget: &Budget<'_>) -> Result<(), NetworkError> {
        Connection::close(self, budget)
    }
}

pub(super) fn exchange(
    connection: &mut impl ResponseChannel,
    bytes: &[u8],
    budget: &Budget<'_>,
    write_budget: &Budget<'_>,
    delivery: &mut Delivery,
    write: &mut ApplicationWrite,
    progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
) -> Result<Response, Error> {
    connection
        .write(bytes, write_budget, write)
        .map_err(|error| {
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
    while !response.is_finished() {
        let Some(bytes) = connection.read(budget).map_err(|error| Error::Transport {
            stage: RequestStage::ResponseRead,
            error,
            delivery: *delivery,
            accepted_wire_bytes: write.accepted_wire_bytes,
        })?
        else {
            break;
        };
        response
            .push(&bytes.bytes, &mut progress)
            .map_err(|error| {
                if response.stream_started() {
                    *delivery = Delivery::Streaming;
                }
                Error::Response {
                    error,
                    delivery: *delivery,
                }
            })?;
        if response.stream_started() {
            *delivery = Delivery::Streaming;
        }
    }
    let result = response.finish().map_err(|error| Error::Response {
        error,
        delivery: *delivery,
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
