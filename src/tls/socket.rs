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
    mut bytes: &[u8],
    budget: &Budget<'_>,
) -> Result<(), NetworkError> {
    while !bytes.is_empty() {
        budget.check()?;
        match stream.write(bytes) {
            Ok(0) => return Err(NetworkError::Eof(IoOperation::WriteRecord)),
            Ok(count) => bytes = &bytes[count..],
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
