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
    stream: &mut impl Read,
    mut bytes: &mut [u8],
    budget: &Budget<'_>,
    operation: IoOperation,
    received: &mut usize,
) -> Result<(), NetworkError> {
    while !bytes.is_empty() {
        budget.check()?;
        match stream.read(bytes) {
            Ok(0) => return Err(NetworkError::Eof(operation)),
            Ok(count) => {
                *received = received.saturating_add(count);
                bytes = &mut bytes[count..];
            }
            Err(error) if retryable(&error) => {}
            Err(error) => return Err(NetworkError::io(operation, &error)),
        }
    }
    Ok(())
}
pub(super) fn record(stream: &mut TcpStream, budget: &Budget<'_>) -> Result<Vec<u8>, NetworkError> {
    record_counted(stream, budget, &mut 0)
}
/// Counts bytes returned by local TCP reads, including an incomplete TLS record.
pub(super) fn record_counted(
    stream: &mut TcpStream,
    budget: &Budget<'_>,
    received: &mut usize,
) -> Result<Vec<u8>, NetworkError> {
    record_from(stream, budget, received)
}
fn record_from(
    stream: &mut impl Read,
    budget: &Budget<'_>,
    received: &mut usize,
) -> Result<Vec<u8>, NetworkError> {
    let mut header = [0; 5];
    read(
        stream,
        &mut header,
        budget,
        IoOperation::ReadRecordHeader,
        received,
    )?;
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
        received,
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
    use std::{
        collections::VecDeque,
        io,
        sync::atomic::{AtomicBool, Ordering},
        time::Instant,
    };
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
            deadline: Some(Instant::now() + Duration::from_secs(1)),
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

    struct CompleteThenCancel<'a> {
        bytes: VecDeque<u8>,
        cancelled: &'a AtomicBool,
    }
    impl Read for CompleteThenCancel<'_> {
        fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
            let count = into.len().min(self.bytes.len());
            for slot in &mut into[..count] {
                *slot = self.bytes.pop_front().unwrap();
            }
            if self.bytes.is_empty() {
                self.cancelled.store(true, Ordering::Release);
            }
            Ok(count)
        }
    }
    #[test]
    fn complete_record_is_returned_when_cancellation_follows_last_read() {
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Some(Instant::now() + Duration::from_secs(1)),
        };
        let record = vec![23, 3, 3, 0, 2, 42, 43];
        let mut stream = CompleteThenCancel {
            bytes: record.clone().into(),
            cancelled: &cancelled,
        };
        let mut received = 0;
        assert_eq!(record_from(&mut stream, &budget, &mut received), Ok(record));
        assert_eq!(received, 7);
    }

    #[test]
    fn cancellation_during_fragmented_record_stops_before_another_read() {
        struct PartialThenCancel<'a>(&'a AtomicBool, usize);
        impl Read for PartialThenCancel<'_> {
            fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
                self.1 += 1;
                into[0] = 23;
                self.0.store(true, Ordering::Release);
                Ok(1)
            }
        }
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: None,
        };
        let mut reader = PartialThenCancel(&cancelled, 0);
        let mut received = 0;
        assert_eq!(
            record_from(&mut reader, &budget, &mut received),
            Err(NetworkError::Cancelled)
        );
        assert_eq!((reader.1, received), (1, 1));
    }

    struct ScriptRead(VecDeque<Result<Vec<u8>, io::ErrorKind>>);
    impl Read for ScriptRead {
        fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
            match self.0.pop_front().unwrap() {
                Ok(bytes) => {
                    assert!(bytes.len() <= into.len());
                    into[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Err(kind) => Err(kind.into()),
            }
        }
    }
    #[test]
    fn fragmented_record_and_resets_count_only_locally_received_bytes() {
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Some(Instant::now() + Duration::from_secs(1)),
        };
        for (script, count, operation) in [
            (
                vec![Ok(vec![23, 3]), Err(io::ErrorKind::ConnectionReset)],
                2,
                IoOperation::ReadRecordHeader,
            ),
            (
                vec![
                    Ok(vec![23, 3]),
                    Ok(vec![3, 0, 4]),
                    Ok(vec![42, 43]),
                    Err(io::ErrorKind::ConnectionReset),
                ],
                7,
                IoOperation::ReadRecordBody,
            ),
        ] {
            let mut stream = ScriptRead(script.into());
            let mut received = 0;
            let error = record_from(&mut stream, &budget, &mut received).unwrap_err();
            assert_eq!(received, count);
            assert!(matches!(error, NetworkError::Io(failure)
                if failure.operation == operation && failure.kind == io::ErrorKind::ConnectionReset));
        }
        let mut stream = ScriptRead(
            vec![
                Ok(vec![23]),
                Ok(vec![3]),
                Ok(vec![3]),
                Ok(vec![0]),
                Ok(vec![2]),
                Ok(vec![42]),
                Ok(vec![43]),
            ]
            .into(),
        );
        let mut received = 0;
        assert_eq!(
            record_from(&mut stream, &budget, &mut received),
            Ok(vec![23, 3, 3, 0, 2, 42, 43])
        );
        assert_eq!(received, 7);
    }
    #[test]
    fn cancellation_during_partial_write_keeps_ambiguous_byte_count() {
        struct CancelWrite<'a>(&'a AtomicBool, usize);
        impl Write for CancelWrite<'_> {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                self.1 += 1;
                self.0.store(true, Ordering::Release);
                Ok(3)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Some(Instant::now() + Duration::from_secs(1)),
        };
        let mut writer = CancelWrite(&cancelled, 0);
        let mut accepted = 0;
        assert_eq!(
            write_counted_to(&mut writer, b"request", &budget, &mut accepted),
            Err(NetworkError::Cancelled)
        );
        assert_eq!((accepted, writer.1), (3, 1));
    }
}
