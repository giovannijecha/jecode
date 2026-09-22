//! Bounded TLS presentation-language fields. No unchecked peer offsets.
use crate::tls::Error;

pub(super) struct Cursor<'a>(pub(super) &'a [u8]);
impl<'a> Cursor<'a> {
    pub(super) fn take(&mut self, length: usize) -> Result<&'a [u8], Error> {
        if length > self.0.len() {
            return Err(Error::Malformed);
        }
        let (head, tail) = self.0.split_at(length);
        self.0 = tail;
        Ok(head)
    }
    pub(super) fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    pub(super) fn word(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub(super) fn vector8(&mut self) -> Result<&'a [u8], Error> {
        let length = usize::from(self.byte()?);
        self.take(length)
    }
    pub(super) fn vector16(&mut self) -> Result<&'a [u8], Error> {
        let length = usize::from(self.word()?);
        self.take(length)
    }
    pub(super) fn vector24(&mut self) -> Result<&'a [u8], Error> {
        let length = length24(self.take(3)?);
        self.take(length)
    }
    pub(super) fn end(self) -> Result<(), Error> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Error::Malformed)
        }
    }
}
pub(super) fn length24(bytes: &[u8]) -> usize {
    (usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2])
}
pub(super) fn extensions(bytes: &[u8]) -> Result<Vec<(u16, &[u8])>, Error> {
    let mut input = Cursor(bytes);
    let mut output = Vec::new();
    while !input.0.is_empty() {
        if output.len() >= 64 {
            return Err(Error::Limit);
        }
        let kind = input.word()?;
        if output.iter().any(|(id, _)| *id == kind) {
            return Err(Error::Malformed);
        }
        output.push((kind, input.vector16()?));
    }
    Ok(output)
}
pub(super) fn vector16(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(
        &u16::try_from(value.len())
            .expect("bounded owned field")
            .to_be_bytes(),
    );
    output.extend_from_slice(value);
}
pub(super) fn extension(output: &mut Vec<u8>, kind: u16, value: &[u8]) {
    output.extend_from_slice(&kind.to_be_bytes());
    vector16(output, value);
}
