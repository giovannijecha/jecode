use super::*;
use crate::backend::openrouter::OpenRouter;
use crate::test_support::Directory;
use crate::tools::Tools;
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn thousands_of_stream_snapshots_leave_only_the_latest_for_a_slow_reader() {
    let (sender, receiver) = event_channel();
    let cancellation = Cancellation::default();
    sender
        .send(
            Event::Waiting {
                model: "test".into(),
            },
            &cancellation,
        )
        .unwrap();
    for index in 0..2_000 {
        sender
            .send(
                Event::Streaming {
                    text: index.to_string(),
                },
                &cancellation,
            )
            .unwrap();
    }
    assert!(matches!(receiver.try_recv(), Ok(Event::Waiting { .. })));
    assert_eq!(
        receiver.try_recv(),
        Ok(Event::Streaming {
            text: "1999".into()
        })
    );
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
}

#[test]
fn final_and_reset_events_invalidate_previews_without_reordering_reliable_events() {
    let (sender, receiver) = event_channel();
    let cancellation = Cancellation::default();
    let send = |event| sender.send(event, &cancellation).unwrap();
    send(Event::Streaming {
        text: "before note".into(),
    });
    send(Event::Maintenance {
        text: "note".into(),
    });
    send(Event::Streaming {
        text: "before final".into(),
    });
    send(Event::Message {
        text: "final".into(),
    });
    send(Event::Streaming {
        text: "before wait".into(),
    });
    send(Event::Waiting {
        model: "next".into(),
    });
    send(Event::Streaming {
        text: "before discard".into(),
    });
    send(Event::RequestDiscarded);
    send(Event::Streaming {
        text: "before retry".into(),
    });
    send(Event::Recovering {
        attempt: 1,
        delay: Duration::ZERO,
        error: "offline".into(),
    });
    assert_eq!(
        receiver.try_recv(),
        Ok(Event::Streaming {
            text: "before note".into()
        })
    );
    assert_eq!(
        receiver.try_recv(),
        Ok(Event::Maintenance {
            text: "note".into()
        })
    );
    assert_eq!(
        receiver.try_recv(),
        Ok(Event::Message {
            text: "final".into()
        })
    );
    assert_eq!(
        receiver.try_recv(),
        Ok(Event::Waiting {
            model: "next".into()
        })
    );
    assert_eq!(receiver.try_recv(), Ok(Event::RequestDiscarded));
    assert!(matches!(receiver.try_recv(), Ok(Event::Recovering { .. })));
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
}

#[test]
fn full_reliable_queue_drops_activity_but_unblocks_on_cancellation() {
    let (sender, receiver) = event_channel();
    let cancellation = Cancellation::default();
    for index in 0..EVENT_CAPACITY {
        sender
            .send(
                Event::Maintenance {
                    text: index.to_string(),
                },
                &cancellation,
            )
            .unwrap();
    }
    sender.send(Event::Reasoning, &cancellation).unwrap();
    sender.send(Event::Working, &cancellation).unwrap();
    let blocked = cancellation.clone();
    let task = thread::spawn(move || {
        sender.send(
            Event::Message {
                text: "final".into(),
            },
            &blocked,
        )
    });
    thread::sleep(Duration::from_millis(25));
    assert!(!task.is_finished());
    cancellation.cancel();
    assert_eq!(task.join().unwrap(), Err("Operation cancelled".into()));
    for index in 0..EVENT_CAPACITY {
        assert_eq!(
            receiver.try_recv(),
            Ok(Event::Maintenance {
                text: index.to_string()
            })
        );
    }
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
}

#[test]
fn slow_reader_receives_every_reliable_event_in_order() {
    let (sender, receiver) = event_channel();
    let cancellation = Cancellation::default();
    let task = thread::spawn(move || {
        for index in 0..64 {
            sender
                .send(
                    Event::Maintenance {
                        text: index.to_string(),
                    },
                    &cancellation,
                )
                .unwrap();
        }
    });
    for index in 0..64 {
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)),
            Ok(Event::Maintenance {
                text: index.to_string()
            })
        );
        thread::sleep(Duration::from_millis(1));
    }
    task.join().unwrap();
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
}

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
fn cancelling_a_worker_during_a_stream_cannot_block_shutdown() {
    use crate::test_support::{HttpFixture, Response};
    let chunks = (0..500).map(|_| (Duration::ZERO, "data: {\"choices\":[{\"delta\":{\"content\":\"fragment \"},\"finish_reason\":null}]}\n\n".into())).collect();
    let fixture = HttpFixture::streaming(vec![Response::Stream(chunks)]);
    let directory = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let worker = Worker::start(agent, "stream".into());
    let next = || loop {
        match worker.events.recv_timeout(Duration::from_secs(3)).unwrap() {
            Event::Context(_) | Event::Preparing => {}
            event => break event,
        }
    };
    assert!(matches!(next(), Event::Waiting { .. }));
    assert!(matches!(next(), Event::Streaming { .. }));
    thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    drop(worker);
    assert!(started.elapsed() < Duration::from_secs(3));
    fixture.finish();
}

#[test]
fn cancelling_wakes_a_turn_waiting_for_a_queued_edit() {
    use crate::test_support::{HttpFixture, completion};
    let fixture = HttpFixture::new(vec![(200, completion("Done.", vec![]))]);
    let directory = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let inbox = agent.inbox();
    inbox.update(|entries| {
        let id = entries.push("being edited".into()).unwrap();
        entries.record();
        entries.hold(id).unwrap();
    });
    let worker = Worker::start(agent, "start".into());
    let started = Instant::now();
    while !matches!(
        worker.events.recv_timeout(Duration::from_secs(3)).unwrap(),
        Event::InputHeld
    ) {
        assert!(started.elapsed() < Duration::from_secs(3));
    }
    assert!(!worker.finished());
    let started = Instant::now();
    worker.cancel();
    while !worker.finished() {
        assert!(started.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    }
    let (_, result) = worker.finish().unwrap();
    assert_eq!(result.unwrap_err(), "Operation cancelled");
    assert_eq!(inbox.with(|entries| entries.len()), 1);
    fixture.finish();
}
