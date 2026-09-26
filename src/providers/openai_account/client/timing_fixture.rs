//! An owned, already-buffered transport for controller-level timing tests.
use super::{
    Delivery, Error,
    exchange::{self, ExchangeProgress, ResponseChannel},
    recovery::Trace,
};
use crate::{
    providers::openai_account::{Progress, Response},
    tls::{ApplicationWrite, Budget, ContentType, NetworkError, Plaintext},
};
use std::{
    collections::VecDeque,
    ops::ControlFlow,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

struct Ready {
    chunks: VecDeque<Vec<u8>>,
    reads: Arc<AtomicUsize>,
}
impl ResponseChannel for Ready {
    fn write(
        &mut self,
        bytes: &[u8],
        budget: &Budget<'_>,
        write: &mut ApplicationWrite,
    ) -> Result<(), NetworkError> {
        budget.check()?;
        write.accepted_wire_bytes = bytes.len();
        Ok(())
    }
    fn read(&mut self, budget: &Budget<'_>) -> Result<Option<Plaintext>, NetworkError> {
        budget.check()?;
        self.reads.fetch_add(1, Ordering::Release);
        Ok(self.chunks.pop_front().map(|bytes| Plaintext {
            kind: ContentType::Application,
            bytes,
        }))
    }
    fn close(&mut self, _: &Budget<'_>) -> Result<(), NetworkError> {
        Ok(())
    }
}

pub(crate) fn exchange_ready(
    chunks: Vec<Vec<u8>>,
    budget: &Budget<'_>,
    idle: Duration,
    reads: Arc<AtomicUsize>,
    progress: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    now: impl Fn() -> Instant,
) -> Result<Response, Error> {
    let mut channel = Ready {
        chunks: chunks.into(),
        reads,
    };
    let mut write = ApplicationWrite::default();
    let mut delivery = Delivery::NotSubmitted;
    let mut trace = Trace::default();
    exchange::exchange_with_clock(
        &mut channel,
        b"synthetic request",
        budget,
        exchange::RequestWindows {
            write: budget,
            idle,
        },
        ExchangeProgress {
            delivery: &mut delivery,
            write: &mut write,
            trace: &mut trace,
        },
        progress,
        now,
    )
}
