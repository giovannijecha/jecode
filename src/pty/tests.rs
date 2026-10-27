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
