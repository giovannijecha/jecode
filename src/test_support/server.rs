//! Loopback server lifetime follows its client, including request preparation.

use crate::cancel::Cancellation;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct Server<T> {
    pub endpoint: String,
    stop: Cancellation,
    worker: Option<JoinHandle<T>>,
}

impl<T: Send + 'static> Server<T> {
    pub fn start(handler: impl FnOnce(TcpListener, Cancellation) -> T + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Cancellation::default();
        let stopping = stop.clone();
        let worker = thread::spawn(move || handler(listener, stopping));
        Self {
            endpoint,
            stop,
            worker: Some(worker),
        }
    }

    pub fn finish(mut self) -> T {
        self.stop.cancel();
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("loopback server panicked")
    }
}

impl<T> Drop for Server<T> {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// A slow client may still be preparing or delivering stdin before connecting.
/// Its supervised operation owns that deadline; the fixture ends with the client.
pub fn accept(listener: &TcpListener, stop: &Cancellation) -> io::Result<TcpStream> {
    loop {
        if stop.requested() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "client finished before connecting",
            ));
        }
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                return Ok(stream);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let _ = stop.wait(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

#[test]
fn a_finished_client_releases_the_server_without_connecting() {
    let server = Server::start(|listener, stop| accept(&listener, &stop));
    assert_eq!(
        server.finish().unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}
