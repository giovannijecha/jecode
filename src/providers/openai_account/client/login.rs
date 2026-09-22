use super::{Error, auth, reply::Reply, unix_seconds};
use crate::tls::{Budget, Connection, NetworkError, trust::TrustStore};
use std::{
    ops::ControlFlow,
    thread,
    time::{Duration, Instant},
};

pub(super) fn run(
    trust: &TrustStore,
    budget: &Budget<'_>,
    mut notify: impl FnMut(&str) -> ControlFlow<()>,
) -> Result<auth::Tokens, Error> {
    let initial = exchange(trust, &auth::start_request()?, budget)?;
    if initial.status != 200 {
        return Err(Error::Status(initial.status));
    }
    let mut login = auth::DeviceLogin::begin(initial.status, &initial.body, Instant::now())?;
    // This code is a transient UI notice, never a conversation/history item.
    if notify(login.user_code()).is_break() {
        return Err(NetworkError::Cancelled.into());
    }
    let budget = Budget {
        deadline: budget.deadline.min(login.deadline()),
        cancelled: budget.cancelled,
    };
    loop {
        wait(login.next_poll(), &budget)?;
        let request = login
            .poll_request(Instant::now())?
            .ok_or(auth::Error::State)?;
        let reply = match exchange(trust, &request, &budget) {
            Err(Error::Network(NetworkError::Timeout)) => {
                // Only a device-code poll has this documented retry behavior.
                login.poll_timed_out(Instant::now())?;
                continue;
            }
            result => result?,
        };
        match login.accept_poll(reply.status, &reply.body, Instant::now())? {
            auth::PollOutcome::Waiting => continue,
            auth::PollOutcome::Authorized => break,
        }
    }
    let request = login.exchange_request(Instant::now())?;
    let reply = exchange(trust, &request, &budget)?;
    if reply.status != 200 {
        return Err(Error::Status(reply.status));
    }
    Ok(login.complete(reply.status, &reply.body, Instant::now(), unix_seconds()?)?)
}

fn wait(until: Instant, budget: &Budget<'_>) -> Result<(), Error> {
    loop {
        budget.check()?;
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        thread::sleep(remaining.min(Duration::from_millis(25)));
    }
}

pub(super) fn exchange(
    trust: &TrustStore,
    request: &[u8],
    budget: &Budget<'_>,
) -> Result<Reply, Error> {
    let budget = Budget {
        deadline: budget
            .deadline
            .min(Instant::now() + Duration::from_secs(30)),
        cancelled: budget.cancelled,
    };
    let mut connection = Connection::connect(auth::AUTH_HOST, trust, &budget)?;
    connection.write(request, &budget)?;
    let mut reply = Reply::new();
    while !reply.is_complete() {
        match connection.read(&budget)? {
            Some(bytes) => reply.push(&bytes.bytes)?,
            None => {
                reply.eof()?;
                break;
            }
        }
    }
    // Drop closes the single-use connection on every success/error path.
    reply.finish()
}
