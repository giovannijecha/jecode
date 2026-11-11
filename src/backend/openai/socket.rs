//! The Responses WebSocket: one connection per conversation on its own adapter
//! process. Every request carries the full history, so nothing depends on
//! connection state, and HTTP stays the fallback. Dated transport measurements
//! are recorded in `docs/reports/PERFORMANCE_RESULTS.md`.

use super::stream::Stream;
use super::{Api, Credential};
use crate::backend::{Failure, Update};
use crate::cancel::Cancellation;
use crate::http::adapter::{Adapter, Frame};
use crate::json::Value;
use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BETA: &str = "responses_websockets=2026-02-06";
const CONNECT_LIMIT: Duration = Duration::from_secs(15);
const SILENCE_LIMIT: Duration = Duration::from_secs(120);
/// The provider ends connections at 60 minutes. Replacing them first keeps that
/// refusal away from an active request.
const LIFETIME: Duration = Duration::from_secs(55 * 60);
/// After a refusal or repeated closes this conversation uses HTTP for a while,
/// instead of paying for a failed connection on every request.
const REFUSAL_PAUSE: Duration = Duration::from_secs(5 * 60);
/// Milliseconds. A finished collaborator keeps its backend, so without this it
/// would hold a process and a provider connection it may never use again.
const IDLE_LIMIT: &str = "300000";
const POLL: Duration = Duration::from_millis(10);

#[derive(Default)]
pub(super) struct Channel {
    connection: Option<Connection>,
    paused_until: Option<Instant>,
}

struct Connection {
    adapter: Adapter,
    opened: Instant,
    /// The upgrade authenticates once, with this token.
    token: String,
}

enum Round {
    /// The stream holds a terminal event.
    Ended,
    /// The connection ended before answering; nothing reached the caller.
    Stale,
    Failed(Failure),
}

/// Sends one request over the conversation's connection. `Ok(false)` leaves the
/// request for HTTP; the stream is then unused.
pub(super) fn exchange(
    channel: &Mutex<Channel>,
    api: &Api,
    conversation: Option<&str>,
    mut request: Value,
    stream: &mut Stream,
    emit: &mut dyn FnMut(Update) -> Result<(), String>,
) -> Result<bool, Failure> {
    let Some(conversation) = conversation.filter(|_| api.sockets) else {
        return Ok(false);
    };
    // A sample overlapping another uses HTTP rather than a second lane.
    let Ok(mut channel) = channel.try_lock() else {
        return Ok(false);
    };
    if channel
        .paused_until
        .is_some_and(|until| Instant::now() < until)
    {
        return Ok(false);
    }
    let message = Arc::new(message(&mut request));
    // A connection that ended while idle gets one fresh connection.
    for _ in 0..2 {
        if api.cancellation.requested() {
            return Err(Failure::from("Operation cancelled"));
        }
        // Asked per request, as HTTP does, so that a sign-out or a refreshed
        // token also ends the connection authenticated with the old one.
        let token = match api.bearer() {
            Ok(token) => token,
            Err(failure) => {
                channel.discard();
                return Err(failure);
            }
        };
        let Some(connection) = channel.ready(api, conversation, token) else {
            if api.cancellation.requested() {
                return Err(Failure::from("Operation cancelled"));
            }
            break;
        };
        emit(Update::Transport("socket")).map_err(Failure::from)?;
        match connection.round(&message, &api.cancellation, stream, emit) {
            Round::Ended => return Ok(true),
            Round::Stale => {
                channel.discard();
                *stream = Stream::default();
            }
            Round::Failed(failure) => {
                channel.discard();
                return Err(failure);
            }
        }
    }
    channel.discard();
    channel.paused_until = Some(Instant::now() + REFUSAL_PAUSE);
    Ok(false)
}

/// The adapter's framing around a `response.create` event. `stream` is a
/// transport field the WebSocket does not use.
fn message(request: &mut Value) -> Vec<u8> {
    if let Value::Object(fields) = request {
        fields.remove("stream");
        fields.insert("type".into(), Value::string("response.create"));
    }
    let event = request.encode();
    let mut framed = format!("{}\n", event.len()).into_bytes();
    framed.extend_from_slice(event.as_bytes());
    framed
}

impl Channel {
    pub(super) fn reset(&mut self) {
        self.discard();
        self.paused_until = None;
    }

    fn ready(&mut self, api: &Api, conversation: &str, token: String) -> Option<&mut Connection> {
        if self
            .connection
            .as_mut()
            .is_some_and(|connection| connection.token == token && connection.usable())
        {
            return self.connection.as_mut();
        }
        self.discard();
        self.connection = Connection::open(api, conversation, token);
        self.connection.as_mut()
    }

    fn discard(&mut self) {
        if let Some(connection) = self.connection.take() {
            connection.adapter.stop();
        }
    }
}

impl Connection {
    fn open(api: &Api, conversation: &str, token: String) -> Option<Self> {
        let authorization = format!("Bearer {token}");
        let mut headers = vec![
            ("Authorization", authorization.as_str()),
            ("OpenAI-Beta", BETA),
        ];
        if matches!(api.credential, Credential::Account(_)) {
            headers.push(("session-id", conversation));
        }
        let url = format!("{}/responses", api.base_url);
        let opening = crate::http::socket_opening(&url, &headers);
        let mut adapter = Adapter::start(&["socket", IDLE_LIMIT]).ok()?;
        let Some(delivery) = adapter.deliver(Arc::new(opening)) else {
            adapter.stop();
            return None;
        };
        let opened = Instant::now();
        loop {
            if api.cancellation.requested() || opened.elapsed() >= CONNECT_LIMIT {
                adapter.stop();
                return None;
            }
            match adapter.frames.recv_timeout(POLL) {
                Ok(Frame::Accepted) => {
                    return if adapter.recover(delivery) {
                        Some(Self {
                            adapter,
                            opened,
                            token,
                        })
                    } else {
                        adapter.stop();
                        None
                    };
                }
                Ok(Frame::End(_) | Frame::RetryableEnd(_))
                | Err(RecvTimeoutError::Disconnected) => {
                    adapter.stop();
                    return None;
                }
                Ok(Frame::Data(_)) | Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn usable(&mut self) -> bool {
        if self.opened.elapsed() >= LIFETIME {
            return false;
        }
        // The provider may send timing or close frames after a terminal event.
        loop {
            match self.adapter.frames.try_recv() {
                Ok(Frame::End(_) | Frame::RetryableEnd(_)) | Err(TryRecvError::Disconnected) => {
                    return false;
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => return self.adapter.alive(),
            }
        }
    }

    fn round(
        &mut self,
        message: &Arc<Vec<u8>>,
        cancellation: &Cancellation,
        stream: &mut Stream,
        emit: &mut dyn FnMut(Update) -> Result<(), String>,
    ) -> Round {
        let Some(delivery) = self.adapter.deliver(Arc::clone(message)) else {
            return Round::Stale;
        };
        let mut delivery = Some(delivery);
        let mut last_activity = Instant::now();
        loop {
            if cancellation.requested() {
                return Round::Failed(Failure::from("Operation cancelled"));
            }
            if last_activity.elapsed() >= SILENCE_LIMIT {
                return Round::Failed(Failure::temporary("OpenAI connection became inactive"));
            }
            match self.adapter.frames.recv_timeout(POLL) {
                Ok(Frame::Data(bytes)) => {
                    last_activity = Instant::now();
                    let Ok(text) = std::str::from_utf8(&bytes) else {
                        return Round::Failed(Failure::temporary("OpenAI returned invalid UTF-8"));
                    };
                    if let Err(error) = stream.message(text, emit) {
                        return Round::Failed(Failure::from(error));
                    }
                    if stream.connection_expired() {
                        return Round::Stale;
                    }
                    if stream.ended() {
                        // Answered, so the adapter read the whole request; a
                        // failed delivery shows on the next `deliver`.
                        if let Some(delivery) = delivery.take() {
                            self.adapter.recover(delivery);
                        }
                        return Round::Ended;
                    }
                }
                Ok(Frame::End(_) | Frame::RetryableEnd(_))
                | Err(RecvTimeoutError::Disconnected) => {
                    return if stream.answering() {
                        Round::Failed(Failure::temporary(
                            "OpenAI connection closed during the response",
                        ))
                    } else {
                        Round::Stale
                    };
                }
                Ok(Frame::Accepted) | Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
