//! Joined, blocking owned TLS over standard-library TCP. No implicit retries.
use super::{
    Error, Plaintext, ServerFlight,
    application::{Application, Incoming},
    socket,
    trust::TrustStore,
};
use std::{
    fmt,
    net::{Shutdown, TcpStream, ToSocketAddrs},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkError {
    Cancelled,
    Timeout,
    Io,
    Dns,
    Closed,
    Tls(Error),
    Certificate(super::certificate::Error),
    Clock,
}
impl From<Error> for NetworkError {
    fn from(error: Error) -> Self {
        Self::Tls(error)
    }
}
impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "connection cancelled",
            Self::Timeout => "connection deadline exceeded",
            Self::Io => "network I/O failed",
            Self::Dns => "host resolution failed",
            Self::Closed => "connection closed",
            Self::Tls(_) => "secure transport failed",
            Self::Certificate(_) => "server identity verification failed",
            Self::Clock => "invalid system clock",
        })
    }
}
impl std::error::Error for NetworkError {}
pub struct Budget<'a> {
    pub deadline: Instant,
    pub cancelled: &'a AtomicBool,
}
impl Budget<'_> {
    pub fn check(&self) -> Result<(), NetworkError> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(NetworkError::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(NetworkError::Timeout)
        } else {
            Ok(())
        }
    }
}
struct State {
    socket: TcpStream,
    application: Application,
    bytes: usize,
}
impl Drop for State {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}
pub struct Connection(Option<State>);
impl Connection {
    /// Verify the host before sending a Finished or permitting application bytes.
    /// Standard-library DNS is synchronous: cancellation is checked before/after
    /// it, but cannot interrupt the OS resolver. No detached resolver survives.
    pub fn connect(
        host: &str,
        trust: &TrustStore,
        budget: &Budget<'_>,
    ) -> Result<Self, NetworkError> {
        budget.check()?;
        let flight = ServerFlight::start(host)?;
        let addresses: Vec<_> = (host, 443)
            .to_socket_addrs()
            .map_err(|_| NetworkError::Dns)?
            .take(8)
            .collect();
        budget.check()?;
        let mut stream = None;
        for address in addresses {
            budget.check()?;
            let timeout = budget
                .deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(500));
            if let Ok(connected) = TcpStream::connect_timeout(&address, timeout) {
                stream = Some(connected);
                break;
            }
        }
        let stream = stream.ok_or(NetworkError::Io)?;
        let now = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| NetworkError::Clock)?
                .as_secs(),
        )
        .map_err(|_| NetworkError::Clock)?;
        Self::handshake(stream, flight, trust, budget, now)
    }
    fn handshake(
        mut socket: TcpStream,
        mut flight: ServerFlight,
        trust: &TrustStore,
        budget: &Budget<'_>,
        now: i64,
    ) -> Result<Self, NetworkError> {
        socket::configure(&socket)?;
        socket::write(&mut socket, flight.client_hello()?, budget)?;
        while !flight.is_ready_for_verification() {
            flight.push(&socket::record(&mut socket, budget)?)?;
        }
        budget.check()?;
        let (finished, application) = flight.authenticate(trust, now)?;
        budget.check()?;
        socket::write(&mut socket, &finished, budget)?;
        Ok(Self(Some(State {
            socket,
            application,
            bytes: 0,
        })))
    }
    /// A failure consumes this connection, even after a partial request write.
    pub fn write(&mut self, bytes: &[u8], budget: &Budget<'_>) -> Result<(), NetworkError> {
        let mut state = self.0.take().ok_or(NetworkError::Closed)?;
        budget.check()?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(Error::Limit.into());
        }
        for chunk in bytes.chunks(16_384) {
            let record = state.application.send(chunk)?;
            socket::write(&mut state.socket, &record, budget)?;
        }
        self.0 = Some(state);
        Ok(())
    }
    /// None is an authenticated close_notify. TCP EOF is always truncation.
    pub fn read(&mut self, budget: &Budget<'_>) -> Result<Option<Plaintext>, NetworkError> {
        let mut state = self.0.take().ok_or(NetworkError::Closed)?;
        for _ in 0..1024 {
            let record = socket::record(&mut state.socket, budget)?;
            state.bytes += record.len();
            if state.bytes > 128 * 1024 * 1024 {
                return Err(Error::Limit.into());
            }
            match state.application.receive(&record)? {
                Incoming::Data(plain) => {
                    self.0 = Some(state);
                    return Ok(Some(plain));
                }
                Incoming::Reply(reply) => socket::write(&mut state.socket, &reply, budget)?,
                Incoming::Continue => {}
                Incoming::Close => return Ok(None),
            }
        }
        Err(Error::Limit.into())
    }
    pub fn close(&mut self, budget: &Budget<'_>) -> Result<(), NetworkError> {
        if let Some(mut state) = self.0.take() {
            let alert = state.application.close()?;
            socket::write(&mut state.socket, &alert, budget)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
