//! Authenticated application epoch and bounded post-handshake processing.
use super::{ContentType, Error, Plaintext, Receiver, Secret, Sender, schedule::label};
pub(super) struct Application {
    client: Secret,
    server: Secret,
    sender: Sender,
    receiver: Receiver,
    pending: Vec<u8>,
    messages: usize,
}
pub(super) enum Incoming {
    Data(Plaintext),
    Reply(Vec<u8>),
    Continue,
    Close,
}
impl Application {
    pub fn new(client: Secret, server: Secret) -> Self {
        Self {
            sender: Sender::new(&client),
            receiver: Receiver::new(&server),
            client,
            server,
            pending: Vec::new(),
            messages: 0,
        }
    }
    pub fn send(&mut self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        self.sender.seal(ContentType::Application, bytes)
    }
    pub fn close(&mut self) -> Result<Vec<u8>, Error> {
        self.sender.seal(ContentType::Alert, &[1, 0])
    }
    pub fn receive(&mut self, record: &[u8]) -> Result<Incoming, Error> {
        let decoded = self.receiver.open(record)?;
        match decoded.kind {
            ContentType::Application if self.pending.is_empty() => Ok(Incoming::Data(decoded)),
            ContentType::Alert if self.pending.is_empty() => {
                if decoded.bytes.len() == 2
                    && matches!(decoded.bytes[0], 1 | 2)
                    && decoded.bytes[1] == 0
                {
                    Ok(Incoming::Close)
                } else {
                    Err(Error::PeerAlert)
                }
            }
            ContentType::Handshake => self.handshake(&decoded.bytes),
            _ => Err(Error::UnexpectedMessage),
        }
    }
    fn handshake(&mut self, bytes: &[u8]) -> Result<Incoming, Error> {
        if self.pending.len() + bytes.len() > 65_792 {
            return Err(Error::Limit);
        }
        self.pending.extend_from_slice(bytes);
        while self.pending.len() >= 4 {
            let kind = self.pending[0];
            let length = (usize::from(self.pending[1]) << 16)
                | (usize::from(self.pending[2]) << 8)
                | usize::from(self.pending[3]);
            if (kind == 24 && length != 1) || !matches!(kind, 4 | 24) {
                return Err(Error::UnexpectedMessage);
            }
            if length > 65_788 {
                return Err(Error::Limit);
            }
            if self.pending.len() < length + 4 {
                break;
            }
            self.messages += 1;
            if self.messages > 32 {
                return Err(Error::Limit);
            }
            if kind == 24 {
                if self.pending.len() != 5 || self.pending[4] > 1 {
                    return Err(Error::Malformed);
                }
                let requested = self.pending[4] == 1;
                self.pending.clear();
                self.server = label::<32>(self.server.as_bytes(), b"traffic upd", &[]);
                self.receiver = Receiver::new(&self.server);
                if requested {
                    let reply = self
                        .sender
                        .seal(ContentType::Handshake, &[24, 0, 0, 1, 0])?;
                    self.client = label::<32>(self.client.as_bytes(), b"traffic upd", &[]);
                    self.sender = Sender::new(&self.client);
                    return Ok(Incoming::Reply(reply));
                }
                return Ok(Incoming::Continue);
            }
            ticket(&self.pending[4..4 + length])?;
            self.pending.drain(..4 + length);
        }
        Ok(Incoming::Continue)
    }
}
fn ticket(body: &[u8]) -> Result<(), Error> {
    // Tickets are deliberately discarded; no PSK, cache or resumption path.
    if body.len() < 13 {
        return Err(Error::Malformed);
    }
    let lifetime = u32::from_be_bytes(body[..4].try_into().unwrap());
    if lifetime > 604_800 {
        return Err(Error::Malformed);
    }
    let mut position = 9 + usize::from(body[8]);
    let take_vector = |position: &mut usize| -> Result<&[u8], Error> {
        let bytes = body.get(*position..*position + 2).ok_or(Error::Malformed)?;
        let length = usize::from(u16::from_be_bytes([bytes[0], bytes[1]]));
        *position += 2;
        let bytes = body
            .get(*position..*position + length)
            .ok_or(Error::Malformed)?;
        *position += length;
        Ok(bytes)
    };
    if take_vector(&mut position)?.is_empty() {
        return Err(Error::Malformed);
    }
    let mut extensions = take_vector(&mut position)?;
    if position != body.len() {
        return Err(Error::Malformed);
    }
    let mut seen = Vec::new();
    while !extensions.is_empty() {
        if extensions.len() < 4 {
            return Err(Error::Malformed);
        }
        let id = u16::from_be_bytes([extensions[0], extensions[1]]);
        let length = usize::from(u16::from_be_bytes([extensions[2], extensions[3]]));
        if seen.contains(&id) || seen.len() >= 64 || extensions.len() < 4 + length {
            return Err(Error::Malformed);
        }
        if id == 42 && length != 4 {
            return Err(Error::Malformed);
        }
        seen.push(id);
        extensions = &extensions[4 + length..];
    }
    Ok(())
}

#[cfg(test)]
mod tests;
