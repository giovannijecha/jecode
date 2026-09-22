use super::{
    AUTH_HOST, CLIENT_ID, Error, REDIRECT_URI, Tokens, form, json_request, parse, required, text,
};
use crate::{
    http,
    json::{self, Value},
};
use std::time::{Duration, Instant};

enum State {
    Ready,
    Polling,
    Granted { code: String, verifier: String },
    Exchanging,
    Closed,
}
#[derive(Debug, PartialEq, Eq)]
pub enum PollOutcome {
    Waiting,
    Authorized,
}

pub struct DeviceLogin {
    device_id: String,
    user_code: String,
    interval: Duration,
    next_poll: Instant,
    deadline: Instant,
    state: State,
}
impl DeviceLogin {
    pub fn begin(status: u16, body: &str, now: Instant) -> Result<Self, Error> {
        if status != 200 {
            return Err(Error::Remote);
        }
        let value = parse(body)?;
        let device_id = required(&value, "device_auth_id", 8192)?.to_owned();
        let key = if value.get("user_code").is_some() {
            "user_code"
        } else {
            "usercode"
        };
        let user_code = required(&value, key, 64)?.to_owned();
        let interval = match value.get("interval") {
            None => 5,
            Some(Value::String(value))
                if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) =>
            {
                value.parse::<u64>().map_err(|_| Error::Invalid)?
            }
            Some(value) => value.unsigned().ok_or(Error::Invalid)?,
        };
        if !(1..=30).contains(&interval) {
            return Err(Error::Invalid);
        }
        let lifetime = match value.get("expires_in") {
            None => 900,
            Some(value) => value
                .unsigned()
                .filter(|n| *n > 0)
                .ok_or(Error::Invalid)?
                .min(900),
        };
        let interval = Duration::from_secs(interval);
        Ok(Self {
            device_id,
            user_code,
            interval,
            next_poll: now.checked_add(interval).ok_or(Error::Invalid)?,
            deadline: now
                .checked_add(Duration::from_secs(lifetime))
                .ok_or(Error::Invalid)?,
            state: State::Ready,
        })
    }
    pub fn user_code(&self) -> &str {
        &self.user_code
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn next_poll(&self) -> Instant {
        self.next_poll
    }
    pub fn cancel(&mut self) {
        self.state = State::Closed;
        self.device_id.clear();
        self.user_code.clear();
    }
    fn check(&mut self, now: Instant) -> Result<(), Error> {
        if matches!(self.state, State::Closed) {
            return Err(Error::Closed);
        }
        if now >= self.deadline {
            self.cancel();
            return Err(Error::Expired);
        }
        Ok(())
    }
    pub fn poll_request(&mut self, now: Instant) -> Result<Option<Vec<u8>>, Error> {
        self.check(now)?;
        if !matches!(self.state, State::Ready) {
            return Err(Error::State);
        }
        if now < self.next_poll {
            return Ok(None);
        }
        let request = json_request(
            "/api/accounts/deviceauth/token",
            json::object([
                ("device_auth_id", text(&self.device_id)),
                ("user_code", text(&self.user_code)),
            ]),
        )?;
        self.state = State::Polling;
        Ok(Some(request))
    }
    pub fn accept_poll(
        &mut self,
        status: u16,
        body: &str,
        now: Instant,
    ) -> Result<PollOutcome, Error> {
        self.check(now)?;
        if !matches!(self.state, State::Polling) {
            return Err(Error::State);
        }
        self.state = State::Closed; // Any unexpected response invalidates this attempt.
        let value = parse(body)?;
        if status == 200 {
            let code = required(&value, "authorization_code", 8192)?.to_owned();
            let verifier = required(&value, "code_verifier", 128)?;
            if verifier.len() < 43
                || !verifier
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
            {
                return Err(Error::Invalid);
            }
            self.state = State::Granted {
                code,
                verifier: verifier.to_owned(),
            };
            return Ok(PollOutcome::Authorized);
        }
        let error = match value.get("error").or_else(|| value.get("code")) {
            None => None,
            Some(Value::String(code)) => Some(code.as_str()),
            Some(Value::Object(fields)) => Some(
                fields
                    .get("code")
                    .and_then(Value::text)
                    .ok_or(Error::Invalid)?,
            ),
            _ => return Err(Error::Invalid),
        };
        if error == Some("access_denied") {
            return Err(Error::Denied);
        }
        if error == Some("expired_token") {
            return Err(Error::Expired);
        }
        let pending = matches!(status, 400 | 403 | 404 | 429)
            && matches!(
                error,
                Some("authorization_pending" | "deviceauth_authorization_pending")
            )
            || matches!(status, 403 | 404) && error.is_none();
        let slower = matches!(status, 400 | 403 | 404 | 429) && error == Some("slow_down")
            || status == 429 && error.is_none();
        if !pending && !slower {
            return Err(Error::Remote);
        }
        if slower {
            self.interval += Duration::from_secs(5);
        }
        self.next_poll = now.checked_add(self.interval).ok_or(Error::Invalid)?;
        self.state = State::Ready;
        Ok(PollOutcome::Waiting)
    }
    /// A timed-out poll may be repeated, with backoff. Token exchanges are never replayed here.
    pub fn poll_timed_out(&mut self, now: Instant) -> Result<(), Error> {
        self.check(now)?;
        if !matches!(self.state, State::Polling) {
            return Err(Error::State);
        }
        self.interval = self
            .interval
            .saturating_mul(2)
            .min(Duration::from_secs(900));
        self.next_poll = now.checked_add(self.interval).ok_or(Error::Invalid)?;
        self.state = State::Ready;
        Ok(())
    }
    pub fn exchange_request(&mut self, now: Instant) -> Result<Vec<u8>, Error> {
        self.check(now)?;
        if !matches!(self.state, State::Granted { .. }) {
            return Err(Error::State);
        }
        let State::Granted { code, verifier } = std::mem::replace(&mut self.state, State::Closed)
        else {
            return Err(Error::State);
        };
        let body = form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", REDIRECT_URI),
        ]);
        let bytes = http::post_form(
            AUTH_HOST,
            "/oauth/token",
            &[("Accept", "application/json")],
            &body,
            65536,
        )
        .map_err(|_| Error::Invalid)?;
        self.state = State::Exchanging;
        Ok(bytes)
    }
    pub fn complete(
        mut self,
        status: u16,
        body: &str,
        now: Instant,
        unix_seconds: u64,
    ) -> Result<Tokens, Error> {
        self.check(now)?;
        if !matches!(self.state, State::Exchanging) {
            return Err(Error::State);
        }
        if status != 200 {
            return Err(Error::Remote);
        }
        Tokens::parse(body, unix_seconds)
    }
}
