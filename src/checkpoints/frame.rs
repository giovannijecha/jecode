//! Versioned journal framing. Each frame is a 24-byte header followed by the
//! payload: magic `JCKP`, format version, record kind, two reserved bytes,
//! little-endian u64 payload length and a u64 checksum of kind, length and
//! payload. Every record carries the sequence number that names it.
use super::hash::{self, Digest};

pub const MAGIC: [u8; 4] = *b"JCKP";
pub const VERSION: u8 = 1;
pub const HEADER: usize = 24;
/// Bound for metadata records; object frames carry their own length.
pub const METADATA_LIMIT: u64 = 1 << 20;
/// A bounded node fans out to 1024 chunks or child manifests.
pub const MANIFEST_CHILDREN: usize = 1024;
pub const MANIFEST_LIMIT: u64 = (MANIFEST_CHILDREN * 8) as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Object = 1,
    Version = 2,
    Stat = 3,
    Prepare = 4,
    Complete = 5,
    Point = 6,
    Pin = 7,
    Policy = 8,
    Event = 9,
    Coverage = 10,
    Moved = 11,
    Recovery = 12,
}

impl Kind {
    pub fn from(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Object,
            2 => Self::Version,
            3 => Self::Stat,
            4 => Self::Prepare,
            5 => Self::Complete,
            6 => Self::Point,
            7 => Self::Pin,
            8 => Self::Policy,
            9 => Self::Event,
            10 => Self::Coverage,
            11 => Self::Moved,
            12 => Self::Recovery,
            _ => return None,
        })
    }
}

pub struct Header {
    pub kind: u8,
    pub length: u64,
    pub checksum: u64,
}

pub fn header(bytes: &[u8; HEADER]) -> Result<Header, String> {
    if bytes[..4] != MAGIC {
        return Err("bad frame magic".into());
    }
    if bytes[4] != VERSION {
        return Err(format!("unsupported frame version {}", bytes[4]));
    }
    Ok(Header {
        kind: bytes[5],
        length: u64::from_le_bytes(bytes[8..16].try_into().unwrap_or_default()),
        checksum: u64::from_le_bytes(bytes[16..24].try_into().unwrap_or_default()),
    })
}

pub fn checksum(kind: u8, payload: &[u8]) -> u64 {
    hash::checksum(&[&[kind], &(payload.len() as u64).to_le_bytes(), payload])
}

pub fn encode(kind: Kind, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER + payload.len());
    frame.extend_from_slice(&MAGIC);
    frame.extend_from_slice(&[VERSION, kind as u8, 0, 0]);
    frame.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    frame.extend_from_slice(&checksum(kind as u8, payload).to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// Enforces replay's metadata size bound before any transaction bytes are written.
/// Object payloads are skipped here; their checksums are verified on reads.
pub fn validate_lengths(mut frames: &[u8]) -> Result<(), String> {
    while !frames.is_empty() {
        let bytes = frames
            .get(..HEADER)
            .ok_or("Truncated checkpoint frame header")?;
        let header = header(bytes.try_into().map_err(|_| "Invalid checkpoint header")?)?;
        if header.kind != Kind::Object as u8 && header.length > METADATA_LIMIT {
            return Err("Checkpoint metadata exceeds the hard 1 MiB record limit".into());
        }
        let length = usize::try_from(header.length).map_err(|_| "Oversized checkpoint frame")?;
        frames = frames[HEADER..]
            .get(length..)
            .ok_or("Truncated checkpoint frame payload")?;
    }
    Ok(())
}

#[derive(Default)]
pub struct Encoder(pub Vec<u8>);

impl Encoder {
    pub fn u8(&mut self, value: u8) -> &mut Self {
        self.0.push(value);
        self
    }
    pub fn u64(&mut self, value: u64) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }
    pub fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.u64(value.len() as u64);
        self.0.extend_from_slice(value);
        self
    }
    pub fn text(&mut self, value: &str) -> &mut Self {
        self.bytes(value.as_bytes())
    }
    pub fn digest(&mut self, value: Digest) -> &mut Self {
        self.u64(value.0[0]).u64(value.0[1])
    }
}

pub struct Decoder<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or("truncated record")?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }
    pub fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().unwrap_or_default(),
        ))
    }
    pub fn bytes(&mut self) -> Result<&'a [u8], String> {
        let length = usize::try_from(self.u64()?).map_err(|_| "oversized field")?;
        self.take(length)
    }
    pub fn text(&mut self) -> Result<String, String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| "invalid text field".into())
    }
    pub fn digest(&mut self) -> Result<Digest, String> {
        Ok(Digest([self.u64()?, self.u64()?]))
    }
}

/// How an object's bytes are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Full,
    /// Base bytes `[..prefix]`, the literal data, then base `[len - suffix..]`.
    Delta {
        base: u64,
        prefix: u64,
        suffix: u64,
    },
    /// The object payload lists this many u64 chunk object identifiers.
    Chunks {
        count: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectMeta {
    pub id: u64,
    pub digest: Digest,
    pub length: u64,
    pub encoding: Encoding,
}

impl ObjectMeta {
    /// Fixed-size prefix of an object payload, followed by the literal data.
    pub const SIZE: usize = 8 + 16 + 8 + 1 + 24;

    pub fn encode(&self, data: &[u8]) -> Vec<u8> {
        let mut out = Encoder(Vec::with_capacity(Self::SIZE + data.len()));
        out.u64(self.id).digest(self.digest).u64(self.length);
        match self.encoding {
            Encoding::Full => out.u8(0).u64(0).u64(0).u64(0),
            Encoding::Delta {
                base,
                prefix,
                suffix,
            } => out.u8(1).u64(base).u64(prefix).u64(suffix),
            Encoding::Chunks { count } => out.u8(2).u64(count).u64(0).u64(0),
        };
        out.0.extend_from_slice(data);
        encode(Kind::Object, &out.0)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut input = Decoder::new(bytes);
        let id = input.u64()?;
        let digest = input.digest()?;
        let length = input.u64()?;
        let tag = input.u8()?;
        let (base, prefix, suffix) = (input.u64()?, input.u64()?, input.u64()?);
        let encoding = match tag {
            0 => Encoding::Full,
            1 => Encoding::Delta {
                base,
                prefix,
                suffix,
            },
            2 if prefix == 0 && suffix == 0 => Encoding::Chunks { count: base },
            _ => return Err("unknown object encoding".into()),
        };
        Ok(Self {
            id,
            digest,
            length,
            encoding,
        })
    }
}
