use super::pool::{Limits, Pool};
use super::*;
use crate::test_support::server::{self, Server};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Barrier};
use std::time::Instant;

fn input_method(url: &str, method: &str) -> Vec<u8> {
    wire_request(&Request {
        url,
        method,
        headers: &[],
        body: (method == "POST").then_some(b"payload".as_slice()),
        timeout: None,
        idle: None,
        output_limit: 0,
    })
}

fn input(url: &str) -> Vec<u8> {
    input_method(url, "POST")
}

fn limits() -> Limits {
    Limits {
        timeout: Some(Duration::from_secs(20)),
        idle: None,
        output: usize::MAX,
    }
}

fn send_method(pool: &Pool, url: &str, method: &str) -> (Output, Vec<u8>) {
    let mut seen = Vec::new();
    let output = pool
        .run(
            input_method(url, method),
            limits(),
            &Cancellation::default(),
            &mut |bytes| {
                seen.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
    (output, seen)
}

fn send(pool: &Pool, url: &str) -> (Output, Vec<u8>) {
    send_method(pool, url, "POST")
}

/// Reads one request; false when the client closed the connection first.
fn read_request(reader: &mut BufReader<TcpStream>) -> bool {
    let mut length = 0;
    let mut first = true;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return false,
            Ok(_) => {}
        }
        if line == "\r\n" && !first {
            break;
        }
        first = false;
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).is_ok()
}

const KEEP_ALIVE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
const CLOSE: &[u8] = b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok";

fn succeeded(output: &Output, seen: &[u8]) {
    assert_eq!(
        output.exit_code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(seen.ends_with(b"\r\n\r\nok"));
}

#[test]
fn consecutive_requests_share_one_adapter_and_one_connection() {
    let server = Server::start(|listener, stop| {
        let stream = server::accept(&listener, &stop).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut served = 0;
        for _ in 0..3 {
            if !read_request(&mut reader) {
                break;
            }
            writer.write_all(KEEP_ALIVE).unwrap();
            served += 1;
        }
        served
    });
    let pool = Pool::new();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let (output, seen) = send(&pool, &server.endpoint);
        succeeded(&output, &seen);
        ids.push(pool.idle_ids());
    }
    assert_eq!(ids[0].len(), 1);
    assert_eq!(ids[0], ids[1]);
    assert_eq!(ids[1], ids[2]);
    assert_eq!(server.finish(), 3);
}

#[test]
fn an_adapter_that_exited_while_idle_is_replaced_without_failing() {
    let server = Server::start(|listener, stop| {
        for _ in 0..2 {
            let stream = server::accept(&listener, &stop).unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            assert!(read_request(&mut reader));
            writer.write_all(CLOSE).unwrap();
        }
    });
    let pool = Pool::new();
    let (output, seen) = send(&pool, &server.endpoint);
    succeeded(&output, &seen);
    let first = pool.idle_ids();
    pool.exit_idle_for_test();
    let (output, seen) = send(&pool, &server.endpoint);
    succeeded(&output, &seen);
    assert_ne!(first, pool.idle_ids());
    server.finish();
}

#[test]
fn a_safe_request_on_a_reused_connection_dropped_before_the_response_is_sent_again() {
    let server = Server::start(|listener, stop| {
        let stream = server::accept(&listener, &stop).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        assert!(read_request(&mut reader));
        writer.write_all(KEEP_ALIVE).unwrap();
        // The next request arrives as the server closes the idle connection.
        assert!(read_request(&mut reader));
        drop(reader);
        drop(writer);
        let stream = server::accept(&listener, &stop).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        assert!(read_request(&mut reader));
        writer.write_all(CLOSE).unwrap();
    });
    let pool = Pool::new();
    let (output, seen) = send_method(&pool, &server.endpoint, "GET");
    succeeded(&output, &seen);
    let (output, seen) = send_method(&pool, &server.endpoint, "GET");
    succeeded(&output, &seen);
    server.finish();
}

#[test]
fn a_post_processed_on_a_reused_connection_is_not_sent_again() {
    let server = Server::start(|listener, stop| {
        let stream = server::accept(&listener, &stop).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        assert!(read_request(&mut reader));
        writer.write_all(KEEP_ALIVE).unwrap();
        // The second POST has been received and may have taken effect, but its
        // response is lost when the existing connection closes.
        let mut effects = 0;
        assert!(read_request(&mut reader));
        effects += 1;
        drop(reader);
        drop(writer);
        // Reconnecting here would execute the same POST twice.
        (effects, server::accept(&listener, &stop).is_err())
    });
    let pool = Pool::new();
    let (output, seen) = send(&pool, &server.endpoint);
    succeeded(&output, &seen);
    let (output, seen) = send(&pool, &server.endpoint);
    assert_eq!(output.exit_code, Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("Native HTTP"));
    assert!(seen.is_empty());
    assert_eq!(server.finish(), (1, true));
}

#[test]
fn a_new_connection_dropped_before_the_response_fails_once_and_keeps_the_adapter() {
    let server = Server::start(|listener, stop| {
        let stream = server::accept(&listener, &stop).unwrap();
        let mut reader = BufReader::new(stream);
        assert!(read_request(&mut reader));
        drop(reader);
        // A second connection here would mean the request was sent twice.
        server::accept(&listener, &stop).is_err()
    });
    let pool = Pool::new();
    let (output, seen) = send(&pool, &server.endpoint);
    assert_eq!(output.exit_code, Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("Native HTTP"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(seen.is_empty());
    assert_eq!(pool.idle_ids().len(), 1);
    assert!(server.finish());
}

#[test]
fn cancellation_discards_the_adapter_and_the_next_request_recovers() {
    let release = Arc::new(Barrier::new(2));
    let server = Server::start({
        let release = Arc::clone(&release);
        move |listener, stop| {
            let stream = server::accept(&listener, &stop).unwrap();
            let mut reader = BufReader::new(stream);
            assert!(read_request(&mut reader));
            release.wait();
            // Stopping the adapter is what closes this connection.
            assert!(!read_request(&mut reader));
            let stream = server::accept(&listener, &stop).unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            assert!(read_request(&mut reader));
            writer.write_all(CLOSE).unwrap();
        }
    });
    let pool = Pool::new();
    let cancellation = Cancellation::default();
    let canceller = std::thread::spawn({
        let cancellation = cancellation.clone();
        move || {
            release.wait();
            cancellation.cancel();
        }
    });
    let started = Instant::now();
    let output = pool
        .run(
            input(&server.endpoint),
            limits(),
            &cancellation,
            &mut |_| Ok(()),
        )
        .unwrap();
    canceller.join().unwrap();
    assert!(output.cancelled);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(pool.idle_ids().is_empty());
    let (output, seen) = send(&pool, &server.endpoint);
    succeeded(&output, &seen);
    server.finish();
}

#[test]
fn silence_past_the_idle_limit_discards_the_adapter() {
    let server = Server::start(|listener, stop| {
        let stream = server::accept(&listener, &stop).unwrap();
        let mut reader = BufReader::new(stream);
        assert!(read_request(&mut reader));
        let _ = stop.wait(Duration::from_secs(30));
    });
    let pool = Pool::new();
    let output = pool
        .run(
            input(&server.endpoint),
            Limits {
                timeout: None,
                idle: Some(Duration::from_millis(300)),
                output: 0,
            },
            &Cancellation::default(),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert!(output.timed_out);
    assert_eq!(output.exit_code, None);
    assert!(pool.idle_ids().is_empty());
    server.finish();
}

#[test]
fn an_observer_failure_stops_the_request_and_is_returned() {
    let server = Server::start(|listener, stop| {
        let stream = server::accept(&listener, &stop).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        assert!(read_request(&mut reader));
        writer.write_all(CLOSE).unwrap();
    });
    let pool = Pool::new();
    let result = pool.run(
        input(&server.endpoint),
        limits(),
        &Cancellation::default(),
        &mut |_| Err("rejected".into()),
    );
    assert_eq!(result.err().as_deref(), Some("rejected"));
    assert!(pool.idle_ids().is_empty());
    server.finish();
}

#[test]
fn the_output_limit_bounds_the_report_but_not_the_observer() {
    let server = Server::start(|listener, stop| {
        let stream = server::accept(&listener, &stop).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        assert!(read_request(&mut reader));
        writer.write_all(CLOSE).unwrap();
    });
    let pool = Pool::new();
    let mut seen = Vec::new();
    let output = pool
        .run(
            input(&server.endpoint),
            Limits {
                timeout: Some(Duration::from_secs(20)),
                idle: None,
                output: 4,
            },
            &Cancellation::default(),
            &mut |bytes| {
                seen.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(output.stdout.len(), 4);
    assert!(output.stdout_truncated);
    assert_eq!(output.stdout_bytes, seen.len());
    assert!(seen.ends_with(b"ok"));
    server.finish();
}

#[test]
fn parallel_requests_use_separate_adapters() {
    let server = Server::start(|listener, stop| {
        // Answering only after both arrive proves they were in flight together.
        let mut streams = Vec::new();
        for _ in 0..2 {
            let stream = server::accept(&listener, &stop).unwrap();
            let writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            assert!(read_request(&mut reader));
            streams.push(writer);
        }
        for mut stream in streams {
            stream.write_all(CLOSE).unwrap();
        }
    });
    let pool = Pool::new();
    let endpoint = server.endpoint.clone();
    std::thread::scope(|scope| {
        for _ in 0..2 {
            let pool = &pool;
            let endpoint = &endpoint;
            scope.spawn(move || {
                let (output, seen) = send(pool, endpoint);
                succeeded(&output, &seen);
            });
        }
    });
    let ids = pool.idle_ids();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    server.finish();
}
