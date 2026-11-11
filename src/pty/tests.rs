#[cfg(unix)]
#[test]
fn timed_out_input_does_not_poison_writer() {
    use super::*;
    use std::io::Read;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    let (socket, mut reader) = UnixStream::pair().unwrap();
    socket.set_nonblocking(true).unwrap();
    reader
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let file = File::from(OwnedFd::from(socket));
    let (sender, receiver) = mpsc::sync_channel(2);
    let failure = Arc::new(Mutex::new(None));
    let closed = Arc::new(AtomicBool::new(false));
    let failed = Arc::clone(&failure);
    let shutdown = Arc::clone(&closed);
    let task = thread::spawn(move || writer(file, receiver, failed, shutdown));
    let (first_sender, first_receipt) = mpsc::sync_channel(1);
    sender
        .send(Message {
            bytes: vec![b'x'; 1024 * 1024],
            receipt: Some(first_sender),
        })
        .unwrap();
    assert!(matches!(
        first_receipt.recv_timeout(DELIVERY + Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(failure.lock().unwrap().is_none());
    let mut bytes = vec![0; 1024 * 1024];
    reader.read_exact(&mut bytes).unwrap();
    assert!(first_receipt.recv_timeout(DELIVERY).unwrap().is_ok());
    let (next_sender, next_receipt) = mpsc::sync_channel(1);
    sender
        .send(Message {
            bytes: b"NEXT".to_vec(),
            receipt: Some(next_sender),
        })
        .unwrap();
    assert!(next_receipt.recv_timeout(DELIVERY).unwrap().is_ok());
    let mut next = [0; 4];
    reader.read_exact(&mut next).unwrap();
    assert_eq!(&next, b"NEXT");
    drop(sender);
    task.join().unwrap();
}

/// A terminal control whose input writer feeds a local pipe, for checking
/// what `write` sends without a native terminal.
fn piped() -> (std::sync::Arc<super::Control>, std::io::PipeReader) {
    use super::*;
    let (reader, pipe) = std::io::pipe().unwrap();
    #[cfg(windows)]
    let file = File::from(std::os::windows::io::OwnedHandle::from(pipe));
    #[cfg(unix)]
    let file = File::from(std::os::fd::OwnedFd::from(pipe));
    let (sender, receiver) = mpsc::sync_channel::<Message>(16);
    let (controls, _) = mpsc::sync_channel::<Message>(16);
    let failure = Arc::new(Mutex::new(None));
    let closed = Arc::new(AtomicBool::new(false));
    let (failed, shutdown) = (Arc::clone(&failure), Arc::clone(&closed));
    thread::spawn(move || writer(file, receiver, failed, shutdown));
    let control = Control {
        writing: Mutex::new(()),
        sender: Mutex::new(Some(sender)),
        controls: Mutex::new(Some(controls)),
        failure,
        closed,
        size: Mutex::new((80, 24)),
        #[cfg(unix)]
        generation: Mutex::new(0),
        _endpoint: Endpoint::new().unwrap(),
    };
    (Arc::new(control), reader)
}

fn queued() -> (
    std::sync::Arc<super::Control>,
    std::sync::mpsc::Receiver<super::Message>,
) {
    use super::*;
    let (sender, receiver) = mpsc::sync_channel(16);
    let (controls, _) = mpsc::sync_channel(16);
    (
        Arc::new(Control {
            writing: Mutex::new(()),
            sender: Mutex::new(Some(sender)),
            controls: Mutex::new(Some(controls)),
            failure: Arc::new(Mutex::new(None)),
            closed: Arc::new(AtomicBool::new(false)),
            size: Mutex::new((80, 24)),
            #[cfg(unix)]
            generation: Mutex::new(0),
            _endpoint: Endpoint::new().unwrap(),
        }),
        receiver,
    )
}

#[test]
fn empty_write_checks_cancellation_and_terminal_end() {
    let (control, _) = queued();
    let cancellation = crate::cancel::Cancellation::default();
    cancellation.cancel();
    assert_eq!(
        control.write(&[], &cancellation).unwrap_err(),
        "Operation cancelled"
    );
    control.close();
    assert_eq!(
        control
            .write(&[], &crate::cancel::Cancellation::default())
            .unwrap_err(),
        "Terminal has ended"
    );
}

#[test]
fn empty_write_waits_for_a_receipt_without_sending_bytes() {
    let (control, receiver) = queued();
    let sending = std::sync::Arc::clone(&control);
    let task =
        std::thread::spawn(move || sending.write(&[], &crate::cancel::Cancellation::default()));
    let message = receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    assert!(message.bytes.is_empty());
    message.receipt.unwrap().send(Ok(())).unwrap();
    task.join().unwrap().unwrap();
}

#[test]
fn direct_input_cannot_interleave_a_segmented_write_or_report_false_success() {
    use super::*;
    let (control, receiver) = queued();
    let input = pattern(SEGMENT + 7);
    let sending = Arc::clone(&control);
    let expected = input.clone();
    let task = thread::spawn(move || sending.write(&expected, &Cancellation::default()));
    let first = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(payload(&first.bytes), input[..SEGMENT]);
    // The first segment cannot be confirmed yet. Direct input must either
    // follow the complete write or report that it was not queued.
    let started = Instant::now();
    let error = control.write_direct(b"key").unwrap_err();
    assert!(error.contains("busy"), "{error}");
    assert!(started.elapsed() <= DIRECT_WAIT + Duration::from_secs(1));
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    first.receipt.unwrap().send(Ok(())).unwrap();
    let second = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(payload(&second.bytes), input[SEGMENT..]);
    second.receipt.unwrap().send(Ok(())).unwrap();
    task.join().unwrap().unwrap();
    control.write_direct(b"key").unwrap();
    let direct = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(payload(&direct.bytes), b"key");
}

#[test]
fn waiting_for_another_write_observes_cancellation() {
    use super::*;
    let (control, receiver) = queued();
    let sending = Arc::clone(&control);
    let first = thread::spawn(move || sending.write(b"first", &Cancellation::default()));
    let pending = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    let cancellation = Arc::new(Cancellation::default());
    let waiting = Arc::clone(&control);
    let cancelled = Arc::clone(&cancellation);
    let second = thread::spawn(move || waiting.write(b"second", &cancelled));
    thread::sleep(Duration::from_millis(20));
    cancellation.cancel();
    assert_eq!(second.join().unwrap().unwrap_err(), "Operation cancelled");
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    pending.receipt.unwrap().send(Ok(())).unwrap();
    first.join().unwrap().unwrap();
}

/// The input bytes of every frame in order; on Unix each frame's interrupt
/// generation must be the write's.
fn payload(stream: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut rest = stream;
    while !rest.is_empty() {
        assert_eq!(rest[0], 1, "input frame");
        let length = u32::from_le_bytes(rest[1..5].try_into().unwrap()) as usize;
        let body = &rest[5..5 + length];
        #[cfg(unix)]
        let body = {
            assert_eq!(&body[..8], &0u64.to_le_bytes());
            &body[8..]
        };
        assert!(body.len() <= 4096);
        bytes.extend_from_slice(body);
        rest = &rest[5 + length..];
    }
    bytes
}

fn pattern(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 251) as u8).collect()
}

#[test]
fn input_beyond_one_segment_arrives_whole_and_in_order() {
    use std::io::Read;
    let (control, mut reader) = piped();
    let drain = std::thread::spawn(move || {
        let mut stream = Vec::new();
        reader.read_to_end(&mut stream).unwrap();
        stream
    });
    // Past the former 64 KiB per call, and not a multiple of a segment.
    let input = pattern(5 * super::SEGMENT + 1234);
    control
        .write(&input, &crate::cancel::Cancellation::default())
        .unwrap();
    control.close();
    drop(control);
    assert_eq!(payload(&drain.join().unwrap()), input);
}

#[test]
fn a_stalled_write_reports_what_was_confirmed_and_sends_nothing_after() {
    use std::io::Read;
    let (control, mut reader) = piped();
    let input = pattern(4 * super::SEGMENT);
    // Nothing reads, so a segment stalls past the confirmation window.
    let error = control
        .write(&input, &crate::cancel::Cancellation::default())
        .unwrap_err();
    assert!(error.contains("input may still be delivered"), "{error}");
    let confirmed = error.split_once("The first ").map_or(0, |(_, rest)| {
        rest.split(' ').next().unwrap().parse().unwrap()
    });
    if confirmed > 0 {
        assert!(error.contains("later bytes were not sent"), "{error}");
    }
    let drain = std::thread::spawn(move || {
        let mut stream = Vec::new();
        reader.read_to_end(&mut stream).unwrap();
        stream
    });
    // The unconfirmed segment may still arrive; nothing after it is sent and
    // nothing is resent.
    std::thread::sleep(super::DELIVERY);
    control.close();
    drop(control);
    let received = payload(&drain.join().unwrap());
    assert_eq!(received, input[..confirmed + super::SEGMENT]);

    // A cancelled write sends nothing.
    let (control, mut reader) = piped();
    let cancellation = crate::cancel::Cancellation::default();
    cancellation.cancel();
    let error = control.write(&input, &cancellation).unwrap_err();
    assert_eq!(error, "Operation cancelled");
    control.close();
    drop(control);
    let mut stream = Vec::new();
    reader.read_to_end(&mut stream).unwrap();
    assert!(stream.is_empty());
}
