//! Restricted, bounded DER field reader. Opaque fields are not semantically validated.
use super::Error;
#[derive(Clone, Copy)]
pub(super) struct Element<'a> {
    pub tag: u8,
    pub body: &'a [u8],
    pub encoded: &'a [u8],
}
pub(super) struct Reader<'a>(pub &'a [u8]);
impl<'a> Reader<'a> {
    pub fn read(&mut self) -> Result<Element<'a>, Error> {
        let input = self.0;
        let header = input.get(..2).ok_or(Error::Encoding)?;
        if header[0] == 0 || header[0] & 31 == 31 {
            return Err(Error::Encoding);
        }
        let mut offset = 2;
        let length = if header[1] < 128 {
            usize::from(header[1])
        } else {
            let count = usize::from(header[1] & 127);
            if count == 0 || count > 3 {
                return Err(Error::Encoding);
            }
            let bytes = input.get(2..2 + count).ok_or(Error::Encoding)?;
            if bytes[0] == 0 {
                return Err(Error::Encoding);
            }
            offset += count;
            let value = bytes
                .iter()
                .fold(0usize, |sum, b| sum * 256 + usize::from(*b));
            if value < 128 {
                return Err(Error::Encoding);
            }
            value
        };
        let encoded = input.get(..offset + length).ok_or(Error::Encoding)?;
        self.0 = &input[encoded.len()..];
        Ok(Element {
            tag: header[0],
            body: &encoded[offset..],
            encoded,
        })
    }
    pub fn expect(&mut self, tag: u8) -> Result<Element<'a>, Error> {
        let value = self.read()?;
        if value.tag != tag {
            return Err(Error::Encoding);
        }
        Ok(value)
    }
    pub fn end(self) -> Result<(), Error> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Error::Encoding)
        }
    }
    pub fn sequence(input: &'a [u8]) -> Result<Self, Error> {
        let mut outer = Self(input);
        let sequence = outer.expect(0x30)?;
        outer.end()?;
        Ok(Self(sequence.body))
    }
    pub fn oid(&mut self) -> Result<&'a [u8], Error> {
        let value = self.expect(6)?.body;
        if value.is_empty() || value.len() > 64 {
            return Err(Error::Encoding);
        }
        let mut start = true;
        for byte in value {
            if start && *byte == 0x80 {
                return Err(Error::Encoding);
            }
            start = byte & 128 == 0;
        }
        if !start {
            return Err(Error::Encoding);
        }
        Ok(value)
    }
    pub fn integer(&mut self) -> Result<&'a [u8], Error> {
        let value = self.expect(2)?.body;
        if value.is_empty()
            || value[0] & 128 != 0
            || (value.len() > 1 && value[0] == 0 && value[1] < 128)
        {
            return Err(Error::Encoding);
        }
        Ok(value)
    }
}
pub(super) fn bits(value: &[u8]) -> Result<(&[u8], u8), Error> {
    let (&unused, data) = value.split_first().ok_or(Error::Encoding)?;
    if unused > 7
        || (data.is_empty() && unused != 0)
        || (unused != 0 && data.last().is_none_or(|b| b & ((1 << unused) - 1) != 0))
    {
        return Err(Error::Encoding);
    }
    Ok((data, unused))
}
