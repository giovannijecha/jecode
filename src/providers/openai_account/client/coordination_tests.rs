use super::{Client, Error, NetworkError, Request, ResponseChannel, auth, persistent};
use crate::{
    providers::openai_account::Input,
    state::Store,
    tls::{Budget, ContentType, Plaintext, trust::TrustStore},
};
use std::{
    ops::ControlFlow,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const ACCESS: &str = "e30.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjb3VudC10ZXN0In19.c2ln";
const COMPLETE: &str = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"synthetic answer\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"synthetic answer\"}]}]}}\n\n";

fn saved(generation: &str) -> String {
    format!(
        r#"{{"version":1,"state":"ready","provider":"openai-account","access_token":"{ACCESS}","refresh_token":"synthetic","account_id":"account-test","expires_at":9999999999,"generation":"{generation}"}}"#
    )
}
fn client(store: &Store, body: &str) -> Client {
    Client {
        trust: TrustStore::native().unwrap(),
        tokens: auth::Tokens::from_saved_json(body).unwrap().unwrap(),
        store: Some(store.clone()),
        catalog: None,
    }
}
fn request() -> Request {
    Request {
        model: "gpt-5.6-luna".into(),
        effort: Some("medium".into()),
        instructions: "synthetic transport fixture".into(),
        input: vec![Input::User("hello".into())],
        tools: Vec::new(),
    }
}
fn budget(cancelled: &AtomicBool, duration: Duration) -> Budget<'_> {
    Budget {
        cancelled,
        deadline: Instant::now() + duration,
    }
}

struct Channel {
    response: Option<Vec<u8>>,
    entered_read: Option<mpsc::Sender<()>>,
    release_read: Option<mpsc::Receiver<()>>,
    writes: Arc<AtomicUsize>,
}
impl Channel {
    fn complete(writes: Arc<AtomicUsize>) -> Self {
        Self {
            response: Some(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{COMPLETE}",
                    COMPLETE.len()
                )
                .into_bytes(),
            ),
            entered_read: None,
            release_read: None,
            writes,
        }
    }
}
impl ResponseChannel for Channel {
    fn write(
        &mut self,
        bytes: &[u8],
        budget: &Budget<'_>,
        progress: &mut crate::tls::ApplicationWrite,
    ) -> Result<(), NetworkError> {
        budget.check()?;
        self.writes.fetch_add(1, Ordering::AcqRel);
        progress.accepted_wire_bytes += bytes.len();
        Ok(())
    }
    fn read(&mut self, _: &Budget<'_>) -> Result<Option<Plaintext>, NetworkError> {
        if let Some(entered) = self.entered_read.take() {
            entered.send(()).unwrap();
        }
        if let Some(release) = self.release_read.take() {
            release
                .recv_timeout(Duration::from_secs(8))
                .map_err(|_| NetworkError::Timeout)?;
        }
        Ok(self.response.take().map(|bytes| Plaintext {
            kind: ContentType::Application,
            bytes,
        }))
    }
    fn close(&mut self, _: &Budget<'_>) -> Result<(), NetworkError> {
        Ok(())
    }
}

#[test]
fn account_replacement_or_logout_before_retry_prevents_another_send() {
    for logout in [false, true] {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else { return };
        let original = saved("a1");
        store.replace("credentials.json", &original).unwrap();
        let mut worker = client(&store, &original);
        let cancelled = AtomicBool::new(false);
        let mut connections = 0;
        let mut attempts = 0;
        let error = worker
            .generate_with::<Channel>(
                &request(),
                &budget(&cancelled, Duration::from_secs(2)),
                |progress| {
                    if let crate::providers::openai_account::Progress::Attempt(attempt) = progress {
                        attempts += 1;
                        assert!(attempt.retrying);
                        if logout {
                            Client::logout_in(
                                store.clone(),
                                &budget(&cancelled, Duration::from_secs(1)),
                            )
                            .unwrap();
                        } else {
                            let replacement = auth::Tokens::from_saved_json(&saved("b2"))
                                .unwrap()
                                .unwrap();
                            super::credentials::Credentials::open(
                                store.clone(),
                                &budget(&cancelled, Duration::from_secs(1)),
                            )
                            .unwrap()
                            .save(&replacement)
                            .unwrap();
                        }
                    }
                    ControlFlow::Continue(())
                },
                |_, _| {
                    connections += 1;
                    Err(NetworkError::io(
                        crate::tls::IoOperation::Connect,
                        &std::io::Error::from(std::io::ErrorKind::ConnectionReset),
                    ))
                },
            )
            .unwrap_err();
        assert_eq!(error, Error::AccountChanged);
        assert_eq!((connections, attempts), (1, 1));
    }
}

#[test]
fn slow_post_connect_account_check_does_not_consume_write_allowance() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else { return };
    let original = saved("a1");
    store.replace("credentials.json", &original).unwrap();
    let mut worker = client(&store, &original);
    let cancelled = AtomicBool::new(false);
    let writes = Arc::new(AtomicUsize::new(0));
    let mut lock_worker = None;
    let response = worker
        .generate_with_timing(
            &request(),
            &budget(&cancelled, Duration::from_secs(2)),
            |_| ControlFlow::Continue(()),
            |_, _| {
                let locked_store = store.clone();
                let (entered, acquired) = mpsc::channel();
                lock_worker = Some(thread::spawn(move || {
                    let local_cancelled = AtomicBool::new(false);
                    let _lock = super::credentials::Credentials::open(
                        locked_store,
                        &budget(&local_cancelled, Duration::from_secs(1)),
                    )
                    .unwrap();
                    entered.send(()).unwrap();
                    thread::sleep(Duration::from_millis(100));
                }));
                acquired.recv_timeout(Duration::from_secs(1)).unwrap();
                Ok(Channel::complete(Arc::clone(&writes)))
            },
            Duration::from_millis(40),
        )
        .unwrap();
    lock_worker.unwrap().join().unwrap();
    assert_eq!(response.text, "synthetic answer");
    assert_eq!(writes.load(Ordering::Acquire), 1);
}

#[test]
fn fresh_clients_generate_independently_and_logout_does_not_revoke_an_in_flight_request() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else {
        return;
    };
    let body = saved("a1");
    store.replace("credentials.json", &body).unwrap();
    let mut first = client(&store, &body);
    let mut second = client(&store, &body);
    let first_writes = Arc::new(AtomicUsize::new(0));
    let (entered, in_read) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let mut channel = Channel::complete(Arc::clone(&first_writes));
    channel.entered_read = Some(entered);
    channel.release_read = Some(released);
    let mut channel = Some(channel);
    let first_request = thread::spawn(move || {
        let cancelled = AtomicBool::new(false);
        first.generate_with(
            &request(),
            &budget(&cancelled, Duration::from_secs(10)),
            |_| ControlFlow::Continue(()),
            |_, _| Ok(channel.take().unwrap()),
        )
    });
    in_read.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(first_writes.load(Ordering::Acquire), 1);

    let cancelled = AtomicBool::new(false);
    let second_writes = Arc::new(AtomicUsize::new(0));
    let response = second
        .generate_with(
            &request(),
            &budget(&cancelled, Duration::from_secs(1)),
            |_| ControlFlow::Continue(()),
            |_, _| Ok(Channel::complete(Arc::clone(&second_writes))),
        )
        .unwrap();
    assert_eq!(response.text, "synthetic answer");
    assert_eq!(second_writes.load(Ordering::Acquire), 1);

    Client::logout_in(store.clone(), &budget(&cancelled, Duration::from_secs(1))).unwrap();
    assert_eq!(
        second
            .generate_with(
                &request(),
                &budget(&cancelled, Duration::from_secs(1)),
                |_| ControlFlow::Continue(()),
                |_, _| -> Result<Channel, NetworkError> { panic!("logged-out client connected") },
            )
            .unwrap_err(),
        Error::AccountChanged
    );
    release.send(()).unwrap();
    assert_eq!(
        first_request.join().unwrap().unwrap().text,
        "synthetic answer"
    );

    let next = saved("b2");
    let next_tokens = auth::Tokens::from_saved_json(&next).unwrap().unwrap();
    super::credentials::Credentials::open(
        store.clone(),
        &budget(&cancelled, Duration::from_secs(1)),
    )
    .unwrap()
    .save(&next_tokens)
    .unwrap();
    assert_eq!(
        second
            .generate_with(
                &request(),
                &budget(&cancelled, Duration::from_secs(1)),
                |_| ControlFlow::Continue(()),
                |_, _| -> Result<Channel, NetworkError> { panic!("stale client connected") },
            )
            .unwrap_err(),
        Error::AccountChanged
    );
}

#[test]
fn logout_during_connection_setup_prevents_a_stale_request_from_being_sent() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else {
        return;
    };
    let body = saved("a1");
    store.replace("credentials.json", &body).unwrap();
    let mut worker = client(&store, &body);
    let writes = Arc::new(AtomicUsize::new(0));
    let (connecting, connected) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let mut channel = Some(Channel::complete(Arc::clone(&writes)));
    let request = thread::spawn(move || {
        let cancelled = AtomicBool::new(false);
        worker.generate_with(
            &request(),
            &budget(&cancelled, Duration::from_secs(5)),
            |_| ControlFlow::Continue(()),
            |_, _| {
                connecting.send(()).unwrap();
                released.recv_timeout(Duration::from_secs(4)).unwrap();
                Ok(channel.take().unwrap())
            },
        )
    });
    connected.recv_timeout(Duration::from_secs(3)).unwrap();
    let cancelled = AtomicBool::new(false);
    Client::logout_in(store, &budget(&cancelled, Duration::from_secs(1))).unwrap();
    release.send(()).unwrap();
    assert_eq!(request.join().unwrap().unwrap_err(), Error::AccountChanged);
    assert_eq!(writes.load(Ordering::Acquire), 0);
}

#[test]
fn login_and_refresh_mutations_finish_before_logout_or_are_rejected_after_it() {
    for refreshing in [false, true] {
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else {
            return;
        };
        let (started, acquired) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let operation_store = store.clone();
        let operation = thread::spawn(move || {
            let cancelled = AtomicBool::new(false);
            let budget = budget(&cancelled, Duration::from_secs(3));
            let credentials =
                super::credentials::Credentials::open(operation_store, &budget).unwrap();
            if refreshing {
                credentials.refreshing().unwrap();
            }
            started.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(2)).unwrap();
            let body = saved(if refreshing { "a1" } else { "b2" });
            credentials
                .save(&auth::Tokens::from_saved_json(&body).unwrap().unwrap())
                .unwrap();
        });
        acquired.recv_timeout(Duration::from_secs(2)).unwrap();
        let logout_store = store.clone();
        let (logout_started, logout_entered) = mpsc::channel();
        let logout = thread::spawn(move || {
            let cancelled = AtomicBool::new(false);
            logout_started.send(()).unwrap();
            Client::logout_in(logout_store, &budget(&cancelled, Duration::from_secs(3)))
        });
        logout_entered.recv_timeout(Duration::from_secs(2)).unwrap();
        release.send(()).unwrap();
        operation.join().unwrap();
        logout.join().unwrap().unwrap();
        let contents = store.read("credentials.json", 32768).unwrap().unwrap();
        assert!(contents.contains("signed_out"));
        assert!(!contents.contains(ACCESS));
    }

    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else {
        return;
    };
    let original = saved("a1");
    store.replace("credentials.json", &original).unwrap();
    let before = store.read("credentials.json", 32768).unwrap().unwrap();
    let cancelled = AtomicBool::new(false);
    let held = super::credentials::Credentials::open(
        store.clone(),
        &budget(&cancelled, Duration::from_secs(3)),
    )
    .unwrap();
    let waiting_store = store.clone();
    let waiting = thread::spawn(move || {
        let cancelled = AtomicBool::new(false);
        persistent::acquire_for_connect(
            waiting_store,
            &budget(&cancelled, Duration::from_secs(3)),
            Some(&before),
        )
        .map(|_| ())
    });
    held.logout().unwrap();
    drop(held);
    assert_eq!(waiting.join().unwrap().unwrap_err(), Error::AccountChanged);
    assert!(
        store
            .read("credentials.json", 32768)
            .unwrap()
            .unwrap()
            .contains("signed_out")
    );
}

#[test]
fn cancelling_while_waiting_for_credential_lock_is_reported_as_cancellation() {
    let fixture = crate::state::tests::Fixture::new();
    let Some(store) = fixture.store() else {
        return;
    };
    let held_cancelled = AtomicBool::new(false);
    let held = super::credentials::Credentials::open(
        store.clone(),
        &budget(&held_cancelled, Duration::from_secs(3)),
    )
    .unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let waiting_cancelled = Arc::clone(&cancelled);
    let (started, entered) = mpsc::channel();
    let waiting = thread::spawn(move || {
        let budget = budget(&waiting_cancelled, Duration::from_secs(3));
        started.send(()).unwrap();
        super::credentials::Credentials::open(store, &budget).map(|_| ())
    });
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    thread::sleep(Duration::from_millis(40));
    assert!(!waiting.is_finished());
    cancelled.store(true, Ordering::Release);
    assert_eq!(
        waiting.join().unwrap().unwrap_err(),
        Error::Network(NetworkError::Cancelled)
    );
    drop(held);
}
