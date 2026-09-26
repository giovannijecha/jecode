//! Joined, blocking owned TLS over standard-library TCP. No implicit retries.
use super::{
    Error, Plaintext, ServerFlight,
    application::{Application, Incoming},
    socket,
    trust::TrustStore,
};
use std::{
    fmt, io,
    net::{Shutdown, TcpStream, ToSocketAddrs},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoOperation {
    Resolve,
    Connect,
    ConfigureRead,
    ConfigureWrite,
    ConfigureNoDelay,
    WriteRecord,
    ReadRecordHeader,
    ReadRecordBody,
}
impl fmt::Display for IoOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Resolve => "host resolution",
            Self::Connect => "TCP connect",
            Self::ConfigureRead => "read timeout setup",
            Self::ConfigureWrite => "write timeout setup",
            Self::ConfigureNoDelay => "TCP option setup",
            Self::WriteRecord => "TLS record write",
            Self::ReadRecordHeader => "TLS record header read",
            Self::ReadRecordBody => "TLS record body read",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoFailure {
    pub operation: IoOperation,
    pub kind: io::ErrorKind,
    pub os_code: Option<i32>,
}
impl IoFailure {
    pub fn new(operation: IoOperation, error: &io::Error) -> Self {
        Self {
            operation,
            kind: error.kind(),
            os_code: error.raw_os_error(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkError {
    Cancelled,
    Timeout,
    Io(IoFailure),
    Dns(IoFailure),
    Eof(IoOperation),
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
impl NetworkError {
    pub fn io(operation: IoOperation, error: &io::Error) -> Self {
        Self::Io(IoFailure::new(operation, error))
    }
}
impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => "connection cancelled",
            Self::Timeout => "connection deadline exceeded",
            Self::Io(failure) | Self::Dns(failure) => {
                write!(
                    f,
                    "network I/O failed during {} ({:?}",
                    failure.operation, failure.kind
                )?;
                if let Some(code) = failure.os_code {
                    write!(f, ", OS {code}")?;
                }
                return f.write_str(")");
            }
            Self::Eof(operation) => return write!(f, "connection ended during {operation}"),
            Self::Closed => "connection closed",
            Self::Tls(_) => "secure transport failed",
            Self::Certificate(_) => "server identity verification failed",
            Self::Clock => "invalid system clock",
        }
        .fmt(f)
    }
}
impl std::error::Error for NetworkError {}
pub struct Budget<'a> {
    /// An optional caller total deadline. Model requests use None; their
    /// transport stages install their own finite deadlines.
    pub deadline: Option<Instant>,
    pub cancelled: &'a AtomicBool,
}
impl Budget<'_> {
    pub fn check(&self) -> Result<(), NetworkError> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(NetworkError::Cancelled)
        } else if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
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
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplicationWrite {
    /// TLS application-record bytes accepted by the local socket. This does
    /// not establish remote receipt, processing, or generation completion.
    pub accepted_wire_bytes: usize,
}
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
            .map_err(|error| NetworkError::Dns(IoFailure::new(IoOperation::Resolve, &error)))?
            .take(8)
            .collect();
        budget.check()?;
        let mut stream = None;
        let mut last_error = None;
        for address in addresses {
            budget.check()?;
            let timeout = budget
                .deadline
                .map_or(Duration::from_millis(500), |deadline| {
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(500))
                });
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(connected) => {
                    stream = Some(connected);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let stream = match stream {
            Some(stream) => stream,
            None => {
                budget.check()?;
                let error =
                    last_error.unwrap_or_else(|| io::Error::from(io::ErrorKind::AddrNotAvailable));
                return Err(NetworkError::io(IoOperation::Connect, &error));
            }
        };
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
        self.write_observed(bytes, budget, &mut ApplicationWrite::default())
    }
    pub fn write_observed(
        &mut self,
        bytes: &[u8],
        budget: &Budget<'_>,
        progress: &mut ApplicationWrite,
    ) -> Result<(), NetworkError> {
        let mut state = self.0.take().ok_or(NetworkError::Closed)?;
        budget.check()?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(Error::Limit.into());
        }
        for chunk in bytes.chunks(16_384) {
            let record = state.application.send(chunk)?;
            socket::write_counted(
                &mut state.socket,
                &record,
                budget,
                &mut progress.accepted_wire_bytes,
            )?;
        }
        self.0 = Some(state);
        Ok(())
    }
    /// None is an authenticated close_notify. TCP EOF is always truncation.
    pub fn read(&mut self, budget: &Budget<'_>) -> Result<Option<Plaintext>, NetworkError> {
        self.read_observed(budget, &mut 0)
    }
    /// The count includes local TCP bytes from incomplete response records.
    pub fn read_observed(
        &mut self,
        budget: &Budget<'_>,
        received_wire_bytes: &mut usize,
    ) -> Result<Option<Plaintext>, NetworkError> {
        let mut state = self.0.take().ok_or(NetworkError::Closed)?;
        for _ in 0..1024 {
            let record = socket::record_counted(&mut state.socket, budget, received_wire_bytes)?;
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
