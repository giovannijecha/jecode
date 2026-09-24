//! Exact record reads avoid consuming bytes from the next TLS key epoch.
use super::{Budget, Error, IoOperation, NetworkError};
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};
pub(super) fn configure(stream: &TcpStream) -> Result<(), NetworkError> {
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(|error| NetworkError::io(IoOperation::ConfigureRead, &error))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(50)))
        .map_err(|error| NetworkError::io(IoOperation::ConfigureWrite, &error))?;
    stream
        .set_nodelay(true)
        .map_err(|error| NetworkError::io(IoOperation::ConfigureNoDelay, &error))
}
pub(super) fn write(
    stream: &mut TcpStream,
    bytes: &[u8],
    budget: &Budget<'_>,
) -> Result<(), NetworkError> {
    write_counted(stream, bytes, budget, &mut 0)
}
/// Counts only bytes of this TLS record accepted by the local socket. The
/// caller decides whether this record carries application data or handshake.
pub(super) fn write_counted(
    stream: &mut TcpStream,
    bytes: &[u8],
    budget: &Budget<'_>,
    accepted: &mut usize,
) -> Result<(), NetworkError> {
    write_counted_to(stream, bytes, budget, accepted)
}
fn write_counted_to(
    stream: &mut impl Write,
    mut bytes: &[u8],
    budget: &Budget<'_>,
    accepted: &mut usize,
) -> Result<(), NetworkError> {
    while !bytes.is_empty() {
        budget.check()?;
        match stream.write(bytes) {
            Ok(0) => return Err(NetworkError::Eof(IoOperation::WriteRecord)),
            Ok(count) => {
                *accepted = accepted.saturating_add(count);
                bytes = &bytes[count..];
            }
            Err(error) if retryable(&error) => {}
            Err(error) => return Err(NetworkError::io(IoOperation::WriteRecord, &error)),
        }
    }
    budget.check()
}

fn read(
    stream: &mut TcpStream,
    mut bytes: &mut [u8],
    budget: &Budget<'_>,
    operation: IoOperation,
) -> Result<(), NetworkError> {
    while !bytes.is_empty() {
        budget.check()?;
        match stream.read(bytes) {
            Ok(0) => return Err(NetworkError::Eof(operation)),
            Ok(count) => bytes = &mut bytes[count..],
            Err(error) if retryable(&error) => {}
            Err(error) => return Err(NetworkError::io(operation, &error)),
        }
    }
    budget.check()
}
pub(super) fn record(stream: &mut TcpStream, budget: &Budget<'_>) -> Result<Vec<u8>, NetworkError> {
    let mut header = [0; 5];
    read(stream, &mut header, budget, IoOperation::ReadRecordHeader)?;
    let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
    if length == 0 || length > 16_401 {
        return Err(Error::Limit.into());
    }
    let mut record = vec![0; 5 + length];
    record[..5].copy_from_slice(&header);
    read(
        stream,
        &mut record[5..],
        budget,
        IoOperation::ReadRecordBody,
    )?;
    Ok(record)
}
fn retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::Interrupted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, io, sync::atomic::AtomicBool, time::Instant};
    struct Script(VecDeque<Result<usize, io::ErrorKind>>);
    impl Write for Script {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .pop_front()
                .unwrap()
                .map(|n| n.min(bytes.len()))
                .map_err(Into::into)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn counted_write_separates_zero_and_partial_socket_acceptance() {
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        for (script, expected) in [
            (vec![Err(io::ErrorKind::ConnectionReset)], 0),
            (vec![Ok(0)], 0),
            (vec![Ok(3), Err(io::ErrorKind::ConnectionReset)], 3),
            (vec![Ok(3), Ok(4)], 7),
        ] {
            let mut writer = Script(script.into());
            let mut accepted = 0;
            let result = write_counted_to(&mut writer, b"request", &budget, &mut accepted);
            assert_eq!(accepted, expected);
            assert_eq!(result.is_ok(), expected == 7);
        }
    }
}
