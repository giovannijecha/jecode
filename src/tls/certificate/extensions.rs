use super::{
    Error,
    der::{Reader, bits},
    names,
};

#[derive(Default)]
pub(super) struct Extensions<'a> {
    pub dns: Vec<&'a str>,
    pub ca: bool,
    pub path_length: Option<u32>,
    pub digital_signature: Option<bool>,
    pub key_cert_sign: Option<bool>,
    pub server_auth: Option<bool>,
    pub unsupported_critical: bool,
    pub unsupported_constraints: bool,
}
impl<'a> Extensions<'a> {
    pub fn parse(input: &'a [u8]) -> Result<Self, Error> {
        let mut list = Reader::sequence(input)?;
        if list.0.is_empty() {
            return Err(Error::Encoding);
        }
        let mut seen = Vec::new();
        let mut output = Self::default();
        while !list.0.is_empty() {
            if seen.len() >= 64 {
                return Err(Error::Limit);
            }
            let mut extension = Reader(list.expect(0x30)?.body);
            let oid = extension.oid()?;
            if seen.contains(&oid) {
                return Err(Error::Encoding);
            }
            seen.push(oid);
            let critical = if extension.0.first() == Some(&1) {
                if extension.expect(1)?.body != [255] {
                    return Err(Error::Encoding);
                }
                true
            } else {
                false
            };
            let value = extension.expect(4)?.body;
            extension.end()?;
            match oid {
                [0x55, 0x1d, 17] => output.san(value)?,
                [0x55, 0x1d, 19] => output.basic(value)?,
                [0x55, 0x1d, 15] => output.usage(value)?,
                [0x55, 0x1d, 37] => output.eku(value)?,
                // These affect path validation even if marked noncritical.
                [0x55, 0x1d, 30 | 33 | 36 | 54] => output.unsupported_constraints = true,
                // TLS Feature may require stapled revocation information. The
                // initial profile does not negotiate or validate OCSP staples.
                [0x2b, 6, 1, 5, 5, 7, 1, 24] => output.unsupported_constraints = true,
                _ if critical => output.unsupported_critical = true,
                _ => {}
            }
        }
        Ok(output)
    }
    fn san(&mut self, value: &'a [u8]) -> Result<(), Error> {
        let mut names = Reader::sequence(value)?;
        let mut count = 0;
        if names.0.is_empty() {
            return Err(Error::Encoding);
        }
        while !names.0.is_empty() {
            count += 1;
            if count > 128 {
                return Err(Error::Limit);
            }
            let item = names.read()?;
            match item.tag {
                0x82 => self.dns.push(names::dns(item.body, true)?),
                0x87 if matches!(item.body.len(), 4 | 16) => {}
                // Other GeneralName choices are deliberately outside this profile.
                _ => return Err(Error::Unsupported),
            }
        }
        Ok(())
    }
    fn basic(&mut self, value: &[u8]) -> Result<(), Error> {
        let mut input = Reader::sequence(value)?;
        if input.0.first() == Some(&1) {
            if input.expect(1)?.body != [255] {
                return Err(Error::Encoding);
            }
            self.ca = true;
        }
        if input.0.first() == Some(&2) {
            let integer = input.integer()?;
            if !self.ca || integer.len() > 4 {
                return Err(Error::Encoding);
            }
            self.path_length = Some(integer.iter().fold(0u32, |n, b| n * 256 + u32::from(*b)));
        }
        input.end()
    }
    fn usage(&mut self, value: &[u8]) -> Result<(), Error> {
        let mut input = Reader(value);
        let (data, unused) = bits(input.expect(3)?.body)?;
        input.end()?;
        if data.is_empty()
            || data.len() > 2
            || data.last() == Some(&0)
            || data.last().unwrap().trailing_zeros() != u32::from(unused)
            || (data.len() == 2 && (data[1] != 0x80 || unused != 7))
        {
            return Err(Error::Encoding);
        }
        self.digital_signature = Some(data[0] & 0x80 != 0);
        if (data[0] & 1 != 0 || data.len() == 2) && data[0] & 8 == 0 {
            return Err(Error::Encoding);
        }
        self.key_cert_sign = Some(data[0] & 4 != 0);
        Ok(())
    }
    fn eku(&mut self, value: &[u8]) -> Result<(), Error> {
        let mut input = Reader::sequence(value)?;
        let mut seen = Vec::new();
        let mut server = false;
        while !input.0.is_empty() {
            if seen.len() >= 32 {
                return Err(Error::Limit);
            }
            let oid = input.oid()?;
            if seen.contains(&oid) {
                return Err(Error::Encoding);
            }
            seen.push(oid);
            server |= oid == [0x2b, 6, 1, 5, 5, 7, 3, 1];
        }
        if seen.is_empty() {
            return Err(Error::Encoding);
        }
        self.server_auth = Some(server);
        Ok(())
    }
}
