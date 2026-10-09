use super::*;
use crate::openrouter::OpenRouter;
use crate::test_support::Directory;
use crate::tools::Tools;
use std::net::TcpListener;
use std::time::{Duration, Instant};

#[test]
fn cancels_a_waiting_native_curl_request_and_returns_the_agent() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/chat/completions", listener.local_addr().unwrap());
    let (accepted, connected) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let server = thread::spawn(move || {
        let started = Instant::now();
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(started.elapsed() < Duration::from_secs(5));
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        };
        accepted.send(()).unwrap();
        let _ = held.recv_timeout(Duration::from_secs(5));
        drop(stream);
    });
    let directory = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture(endpoint),
        Tools::new(directory.path()).unwrap(),
    );
    let worker = Worker::start(agent, "Wait for the fixture.".into());
    connected.recv_timeout(Duration::from_secs(5)).unwrap();
    let started = Instant::now();
    worker.cancel();
    while !worker.finished() {
        assert!(started.elapsed() < Duration::from_secs(4));
        thread::sleep(Duration::from_millis(10));
    }
    let (agent, result) = worker.finish().unwrap();
    assert_eq!(result.unwrap_err(), "Operation cancelled");
    assert_eq!(agent.archive().messages.lock().unwrap().len(), 2);
    release.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn cancelling_a_worker_with_a_full_stream_channel_cannot_block_shutdown() {
    use crate::test_support::{HttpFixture, Response};
    let chunks = (0..500).map(|_| (Duration::ZERO, "data: {\"choices\":[{\"delta\":{\"content\":\"fragment \"},\"finish_reason\":null}]}\n\n".into())).collect();
    let fixture = HttpFixture::streaming(vec![Response::Stream(chunks)]);
    let directory = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let worker = Worker::start(agent, "stream".into());
    assert!(matches!(
        worker.events.recv_timeout(Duration::from_secs(3)).unwrap(),
        Event::Waiting { .. }
    ));
    assert!(matches!(
        worker.events.recv_timeout(Duration::from_secs(3)).unwrap(),
        Event::Streaming { .. }
    ));
    thread::sleep(Duration::from_millis(100));
    assert!(!worker.finished());
    let started = Instant::now();
    drop(worker);
    assert!(started.elapsed() < Duration::from_secs(3));
    fixture.finish();
}
