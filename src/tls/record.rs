//! Complete TLS 1.3 AES-128-GCM records. Framing/IO and handshake order live above.
use super::{
    Secret,
    crypto::{aes_gcm::Key, secret::erase},
    schedule::label,
};

const MAX_CONTENT: usize = 16_384;
const MAX_CIPHERTEXT: usize = MAX_CONTENT + 1 + 16;
// Conservative fixed cap, below RFC 8446's AES-GCM 2^24.5-record guidance.
// Each KeyUpdate installs a fresh epoch. Close if an epoch reaches this cap.
const MAX_RECORDS: u64 = 1 << 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Closed,
    Limit,
    Malformed,
    Authentication,
    Unsupported,
    UnexpectedMessage,
    PeerAlert,
    Truncated,
    Entropy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ContentType {
    Alert = 21,
    Handshake = 22,
    Application = 23,
}

pub struct Plaintext {
    pub kind: ContentType,
    pub bytes: Vec<u8>,
}

impl Drop for Plaintext {
    fn drop(&mut self) {
        erase(&mut self.bytes);
    }
}

struct Keys {
    key: Key,
    iv: [u8; 12],
    sequence: u64,
}
impl Keys {
    fn new(secret: &Secret) -> Self {
        let key = label::<16>(secret.as_bytes(), b"key", &[]);
        let iv = label::<12>(secret.as_bytes(), b"iv", &[]);
        Self {
            key: Key::new(&key.as_bytes()[..16]).expect("AES-128 key"),
            iv: iv.as_bytes()[..12].try_into().unwrap(),
            sequence: 0,
        }
    }
    fn nonce(&self) -> Result<[u8; 12], Error> {
        if self.sequence >= MAX_RECORDS {
            return Err(Error::Limit);
        }
        let mut nonce = self.iv;
        for (a, b) in nonce[4..].iter_mut().zip(self.sequence.to_be_bytes()) {
            *a ^= b;
        }
        Ok(nonce)
    }
}
impl Drop for Keys {
    fn drop(&mut self) {
        erase(&mut self.iv);
    }
}

/// One sending epoch. Never clone, reset or reuse a traffic secret for a new epoch.
pub struct Sender(Option<Keys>);
impl Sender {
    pub fn new(secret: &Secret) -> Self {
        Self(Some(Keys::new(secret)))
    }
    pub fn close(&mut self) {
        self.0 = None;
    }
    /// No padding is added. Failed admission permanently closes this epoch.
    pub fn seal(&mut self, kind: ContentType, content: &[u8]) -> Result<Vec<u8>, Error> {
        let result = self.seal_inner(kind, content);
        if result.is_err() {
            self.close();
        }
        result
    }
    fn seal_inner(&mut self, kind: ContentType, content: &[u8]) -> Result<Vec<u8>, Error> {
        let keys = self.0.as_mut().ok_or(Error::Closed)?;
        if content.len() > MAX_CONTENT {
            return Err(Error::Limit);
        }
        validate_content(kind, content)?;
        let nonce = keys.nonce()?;
        let length = (content.len() + 1 + 16) as u16;
        let header = [23, 3, 3, (length >> 8) as u8, length as u8];
        let mut output = Vec::with_capacity(5 + usize::from(length));
        output.extend_from_slice(&header);
        output.extend_from_slice(content);
        output.push(kind as u8);
        let tag = keys
            .key
            .seal(&nonce, &header, &mut output[5..])
            .map_err(|_| Error::Limit)?;
        output.extend_from_slice(&tag);
        keys.sequence += 1;
        Ok(output)
    }
}

/// One receiving epoch. Any invalid record poisons it; subsequent input is rejected.
pub struct Receiver(Option<Keys>);
impl Receiver {
    pub fn new(secret: &Secret) -> Self {
        Self(Some(Keys::new(secret)))
    }
    pub fn close(&mut self) {
        self.0 = None;
    }
    /// Supply exactly one bounded record. No plaintext is released on failure.
    pub fn open(&mut self, wire: &[u8]) -> Result<Plaintext, Error> {
        let result = self.open_inner(wire);
        if result.is_err() {
            self.close();
        }
        result
    }
    fn open_inner(&mut self, wire: &[u8]) -> Result<Plaintext, Error> {
        let keys = self.0.as_mut().ok_or(Error::Closed)?;
        if wire.len() < 5 || wire[..3] != [23, 3, 3] {
            return Err(Error::Malformed);
        }
        let length = usize::from(u16::from_be_bytes([wire[3], wire[4]]));
        if length > MAX_CIPHERTEXT {
            return Err(Error::Limit);
        }
        if length < 17 || wire.len() != length + 5 {
            return Err(Error::Malformed);
        }
        let nonce = keys.nonce()?;
        // The allocation is bounded before copying. Its clearing owner also covers
        // successfully decrypted bytes rejected by the inner content-type checks.
        let mut plaintext = Plaintext {
            kind: ContentType::Application,
            bytes: wire[5..wire.len() - 16].to_vec(),
        };
        keys.key
            .open(
                &nonce,
                &wire[..5],
                &mut plaintext.bytes,
                &wire[wire.len() - 16..],
            )
            .map_err(|_| Error::Authentication)?;
        let end = plaintext
            .bytes
            .iter()
            .rposition(|byte| *byte != 0)
            .ok_or(Error::Malformed)?;
        plaintext.kind = match plaintext.bytes[end] {
            21 => ContentType::Alert,
            22 => ContentType::Handshake,
            23 => ContentType::Application,
            _ => return Err(Error::Malformed),
        };
        erase(&mut plaintext.bytes[end..]);
        plaintext.bytes.truncate(end);
        validate_content(plaintext.kind, &plaintext.bytes)?;
        keys.sequence += 1;
        Ok(plaintext)
    }
}

fn validate_content(kind: ContentType, content: &[u8]) -> Result<(), Error> {
    if (kind == ContentType::Handshake && content.is_empty())
        || (kind == ContentType::Alert && content.len() != 2)
    {
        return Err(Error::Malformed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_budget_never_wraps_or_restarts() {
        let secret = Secret([1; 32]);
        let mut sender = Sender::new(&secret);
        sender.0.as_mut().unwrap().sequence = MAX_RECORDS;
        assert_eq!(
            sender.seal(ContentType::Application, b"x"),
            Err(Error::Limit)
        );
        assert_eq!(
            sender.seal(ContentType::Application, b"x"),
            Err(Error::Closed)
        );
        let mut receiver = Receiver::new(&secret);
        receiver.0.as_mut().unwrap().sequence = MAX_RECORDS;
        let wire = Sender::new(&secret)
            .seal(ContentType::Application, b"x")
            .unwrap();
        assert!(matches!(receiver.open(&wire), Err(Error::Limit)));
        assert!(matches!(receiver.open(&wire), Err(Error::Closed)));
    }
}
