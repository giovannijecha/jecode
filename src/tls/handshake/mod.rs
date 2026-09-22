//! Offline client-side server flight. Completion still requires peer verification.
mod hello;
mod peer;
mod wire;

use super::{ContentType, Error, HandshakeSecrets, KeyShare, Receiver, crypto::sha256::Sha256};
pub use peer::UnverifiedPeer;

const MAX_WIRE: usize = 1_048_576;
const MAX_RECORDS: usize = 1024;
const MAX_MESSAGE: usize = 262_144;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    Hello,
    Extensions,
    Certificate,
    Verify,
    Finished,
    Unverified,
}

struct State {
    phase: Phase,
    hello: Vec<u8>,
    key: Option<KeyShare>,
    hash: Sha256,
    secrets: Option<HandshakeSecrets>,
    receiver: Option<Receiver>,
    record: Vec<u8>,
    message: Vec<u8>,
    wire_bytes: usize,
    records: usize,
    ccs: usize,
    peer: UnverifiedPeer,
}

/// Accepts arbitrary network chunks, but performs no IO and authenticates no peer.
/// `start` obtains native entropy; `new` accepts explicit material for fixtures.
/// The application transition verifies native trust and the exact peer proof.
pub struct ServerFlight(Option<State>);
impl ServerFlight {
    /// Production entry: obtain fresh key/random bytes from the native OS.
    /// This still performs no networking and returns no authenticated connection.
    pub fn start(host: &str) -> Result<Self, Error> {
        let (key, random) = super::entropy::generate()?;
        Self::new(host, &random, key)
    }

    pub fn new(host: &str, random: &[u8; 32], key: KeyShare) -> Result<Self, Error> {
        let message = hello::client(host, random, &key.public())?;
        let mut hash = Sha256::new();
        hash.update(&message);
        let mut hello = vec![22, 3, 1];
        wire::vector16(&mut hello, &message);
        Ok(Self(Some(State {
            phase: Phase::Hello,
            hello,
            key: Some(key),
            hash,
            secrets: None,
            receiver: None,
            record: Vec::new(),
            message: Vec::new(),
            wire_bytes: 0,
            records: 0,
            ccs: 0,
            peer: UnverifiedPeer {
                host: host.to_ascii_lowercase(),
                certificates: Vec::new(),
                algorithm: 0,
                signature: Vec::new(),
                signed_message: Vec::new(),
            },
        })))
    }
    /// Encode once per new connection. This accessor does not authorize replay.
    pub fn client_hello(&self) -> Result<&[u8], Error> {
        Ok(&self.0.as_ref().ok_or(Error::Closed)?.hello)
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let mut state = self.0.take().ok_or(Error::Closed)?;
        state.push(bytes)?;
        self.0 = Some(state);
        Ok(())
    }
    pub fn is_ready_for_verification(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|state| state.phase == Phase::Unverified)
    }
    pub fn cancel(&mut self) {
        self.0 = None;
    }
    pub(super) fn authenticate(
        mut self,
        trust: &super::trust::TrustStore,
        now: i64,
    ) -> Result<(Vec<u8>, super::application::Application), super::connection::NetworkError> {
        let state = self.0.take().ok_or(Error::Closed)?;
        if state.phase != Phase::Unverified || !state.record.is_empty() || !state.message.is_empty()
        {
            return Err(Error::Truncated.into());
        }
        super::certificate::verify_peer(&state.peer, trust, now)
            .map_err(super::connection::NetworkError::Certificate)?;
        let secrets = state.secrets.ok_or(Error::Closed)?;
        let hash = state.hash.finish();
        let finished = super::Sender::new(&secrets.client)
            .seal(ContentType::Handshake, &secrets.client_finished(&hash))?;
        let (client, server) = secrets.application(&hash);
        Ok((
            finished,
            super::application::Application::new(client, server),
        ))
    }
    /// Consumes the flight and drops handshake keys. It yields only untrusted
    /// public evidence; this is deliberately not a usable TLS connection.
    pub fn finish(mut self) -> Result<UnverifiedPeer, Error> {
        let state = self.0.take().ok_or(Error::Closed)?;
        if state.phase != Phase::Unverified || !state.record.is_empty() || !state.message.is_empty()
        {
            return Err(Error::Truncated);
        }
        Ok(state.peer)
    }
}

impl State {
    fn push(&mut self, mut input: &[u8]) -> Result<(), Error> {
        if self.phase == Phase::Unverified {
            return Err(Error::Closed);
        }
        if input.len() > MAX_WIRE - self.wire_bytes {
            return Err(Error::Limit);
        }
        self.wire_bytes += input.len();
        while !input.is_empty() {
            if self.record.len() < 5 {
                let take = (5 - self.record.len()).min(input.len());
                self.record.extend_from_slice(&input[..take]);
                input = &input[take..];
                if self.record.len() < 5 {
                    break;
                }
            }
            let length = usize::from(u16::from_be_bytes([self.record[3], self.record[4]]));
            let max = if self.record[0] == 23 { 16_401 } else { 16_384 };
            if length == 0 || length > max {
                return Err(Error::Limit);
            }
            let take = (5 + length - self.record.len()).min(input.len());
            self.record.extend_from_slice(&input[..take]);
            input = &input[take..];
            if self.record.len() < length + 5 {
                break;
            }
            self.records += 1;
            if self.records > MAX_RECORDS {
                return Err(Error::Limit);
            }
            let mut record = std::mem::take(&mut self.record);
            self.record(&record)?;
            record.clear();
            self.record = record;
            // Finished is a key-change boundary. This offline collector cannot
            // accept encrypted application data or anything beyond that boundary.
            if self.phase == Phase::Unverified && !input.is_empty() {
                return Err(Error::UnexpectedMessage);
            }
        }
        Ok(())
    }
    fn record(&mut self, record: &[u8]) -> Result<(), Error> {
        if record[1..3] != [3, 3] {
            return Err(Error::Malformed);
        }
        if record[0] == 20 {
            if record[5..] != [1] || !self.message.is_empty() {
                return Err(Error::UnexpectedMessage);
            }
            self.ccs += 1;
            if self.ccs > 2 {
                return Err(Error::Limit);
            }
            return Ok(());
        }
        if self.phase == Phase::Hello {
            match record[0] {
                22 => self.messages(&record[5..]),
                21 if record.len() == 7 => Err(Error::PeerAlert),
                _ => Err(Error::UnexpectedMessage),
            }
        } else {
            let decoded = self.receiver.as_mut().ok_or(Error::Closed)?.open(record)?;
            match decoded.kind {
                ContentType::Handshake => self.messages(&decoded.bytes),
                ContentType::Alert => Err(Error::PeerAlert),
                _ => Err(Error::UnexpectedMessage),
            }
        }
    }
    fn messages(&mut self, mut bytes: &[u8]) -> Result<(), Error> {
        while !bytes.is_empty() {
            if self.message.len() < 4 {
                let take = (4 - self.message.len()).min(bytes.len());
                self.message.extend_from_slice(&bytes[..take]);
                bytes = &bytes[take..];
                if self.message.len() < 4 {
                    return Ok(());
                }
            }
            let expected = match self.phase {
                Phase::Hello => 2,
                Phase::Extensions => 8,
                Phase::Certificate => 11,
                Phase::Verify => 15,
                Phase::Finished => 20,
                Phase::Unverified => return Err(Error::UnexpectedMessage),
            };
            if self.message[0] != expected {
                return Err(Error::UnexpectedMessage);
            }
            let length = wire::length24(&self.message[1..4]);
            let max = if self.phase == Phase::Certificate {
                MAX_MESSAGE
            } else {
                4096
            };
            if length > max {
                return Err(Error::Limit);
            }
            let take = (4 + length - self.message.len()).min(bytes.len());
            self.message.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.message.len() < 4 + length {
                return Ok(());
            }
            let message = std::mem::take(&mut self.message);
            let changing_keys = matches!(self.phase, Phase::Hello | Phase::Finished);
            if changing_keys && !bytes.is_empty() {
                return Err(Error::UnexpectedMessage);
            }
            self.message(&message)?;
        }
        Ok(())
    }
    fn message(&mut self, message: &[u8]) -> Result<(), Error> {
        let body = &message[4..];
        match self.phase {
            Phase::Hello => {
                let public = hello::server(body)?;
                let shared = self
                    .key
                    .take()
                    .ok_or(Error::Closed)?
                    .shared(&public)
                    .map_err(|_| Error::Authentication)?;
                self.hash.update(message);
                let secrets = HandshakeSecrets::derive(&shared, &self.hash.clone().finish());
                self.receiver = Some(Receiver::new(&secrets.server));
                self.secrets = Some(secrets);
                self.phase = Phase::Extensions;
                return Ok(());
            }
            Phase::Extensions => {
                hello::encrypted_extensions(body)?;
                self.phase = Phase::Certificate;
            }
            Phase::Certificate => {
                self.peer.certificates = peer::certificates(body)?;
                self.phase = Phase::Verify;
            }
            Phase::Verify => {
                peer::signature(body, &self.hash.clone().finish(), &mut self.peer)?;
                self.phase = Phase::Finished;
            }
            Phase::Finished => {
                if !self
                    .secrets
                    .as_ref()
                    .ok_or(Error::Closed)?
                    .verify_server_finished(&self.hash.clone().finish(), body)
                {
                    return Err(Error::Authentication);
                }
                self.phase = Phase::Unverified;
            }
            Phase::Unverified => return Err(Error::UnexpectedMessage),
        }
        self.hash.update(message);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
