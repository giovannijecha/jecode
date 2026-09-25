//! Owned PNG container validation and private, content-addressed image evidence.
//! Pixel decompression remains the account provider's responsibility. No image
//! bytes are placed in a transcript or canonical JSON event.
use crate::{
    json::{self, Value},
    state::Store,
    tls::crypto::sha256::Sha256,
    workspace::MAX_IMAGE_BYTES,
};
use std::io::{self, Read};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Evidence {
    pub id: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}

pub(crate) struct Images(Store);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Invalid,
    Unsupported,
    Limit,
    Missing,
    Integrity,
    Storage,
}
impl Error {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Invalid => "invalid PNG image data",
            Self::Unsupported => "unsupported image format; view_image currently accepts PNG",
            Self::Limit => "image exceeds the 5 MiB capture limit; save a smaller PNG and retry",
            Self::Missing => {
                "saved image evidence is missing; restore the session image store from backup"
            }
            Self::Integrity => {
                "saved image evidence failed integrity checks; restore it from backup"
            }
            Self::Storage => "could not durably save image evidence; view_image was not completed",
        }
    }
}

impl Images {
    pub(crate) fn in_store(root: &Store, session: &str) -> io::Result<Self> {
        if !session.starts_with("s-")
            || !session[2..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b'-')
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Self(root.directory("images")?.directory(session)?))
    }

    pub(crate) fn capture(&self, bytes: &[u8], path: &str) -> Result<Evidence, Error> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(Error::Limit);
        }
        let (width, height) = inspect(bytes)?;
        let id = hex(&Sha256::digest(bytes));
        let evidence = Evidence {
            id,
            path: path.into(),
            width,
            height,
            bytes: bytes.len(),
        };
        let name = format!("{}.png", evidence.id);
        match self.0.read_file(&name) {
            Ok(_) => {
                if self.load(&evidence)? != bytes {
                    return Err(Error::Integrity);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.0
                    .replace_bytes(&name, bytes)
                    .map_err(|_| Error::Storage)?;
                if self.load(&evidence)? != bytes {
                    return Err(Error::Integrity);
                }
            }
            Err(_) => return Err(Error::Storage),
        }
        Ok(evidence)
    }

    pub(crate) fn load_id(&self, id: &str) -> Result<Evidence, Error> {
        if !valid_id(id) {
            return Err(Error::Invalid);
        }
        let name = format!("{id}.png");
        let bytes = read(&self.0, &name)?;
        if hex(&Sha256::digest(&bytes)) != id {
            return Err(Error::Integrity);
        }
        let (width, height) = inspect(&bytes).map_err(|_| Error::Integrity)?;
        Ok(Evidence {
            id: id.into(),
            path: format!("saved image {id}"),
            width,
            height,
            bytes: bytes.len(),
        })
    }

    pub(crate) fn load(&self, evidence: &Evidence) -> Result<Vec<u8>, Error> {
        if !valid_id(&evidence.id) {
            return Err(Error::Integrity);
        }
        let bytes = read(&self.0, &format!("{}.png", evidence.id))?;
        if bytes.len() != evidence.bytes || hex(&Sha256::digest(&bytes)) != evidence.id {
            return Err(Error::Integrity);
        }
        let (width, height) = inspect(&bytes).map_err(|_| Error::Integrity)?;
        if (width, height) != (evidence.width, evidence.height) {
            return Err(Error::Integrity);
        }
        Ok(bytes)
    }
}

fn read(store: &Store, name: &str) -> Result<Vec<u8>, Error> {
    let file = store.read_file(name).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Error::Missing
        } else {
            Error::Storage
        }
    })?;
    if file.metadata().map_err(|_| Error::Storage)?.len() > MAX_IMAGE_BYTES as u64 {
        return Err(Error::Integrity);
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Storage)?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(Error::Integrity);
    }
    Ok(bytes)
}

impl Evidence {
    pub(crate) fn value(&self) -> Value {
        json::object([
            ("id", Value::String(self.id.clone())),
            ("path", Value::String(self.path.clone())),
            ("format", Value::String("png".into())),
            ("width", Value::Number(self.width.to_string())),
            ("height", Value::Number(self.height.to_string())),
            ("bytes", Value::Number(self.bytes.to_string())),
        ])
    }
    pub(crate) fn parse(value: &Value) -> io::Result<Self> {
        let invalid = || io::Error::from(io::ErrorKind::InvalidData);
        let id = value
            .get("id")
            .and_then(Value::text)
            .filter(|s| valid_id(s))
            .ok_or_else(invalid)?;
        let path = value
            .get("path")
            .and_then(Value::text)
            .filter(|s| s.len() <= 4096 && !s.chars().any(char::is_control))
            .ok_or_else(invalid)?;
        if value.get("format").and_then(Value::text) != Some("png") {
            return Err(invalid());
        }
        let width = value
            .get("width")
            .and_then(Value::unsigned)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n != 0)
            .ok_or_else(invalid)?;
        let height = value
            .get("height")
            .and_then(Value::unsigned)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n != 0)
            .ok_or_else(invalid)?;
        let bytes = value
            .get("bytes")
            .and_then(Value::unsigned)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= MAX_IMAGE_BYTES)
            .ok_or_else(invalid)?;
        Ok(Self {
            id: id.into(),
            path: path.into(),
            width,
            height,
            bytes,
        })
    }
    pub(crate) fn description(&self) -> String {
        format!(
            "PNG {}x{}, {} bytes, image_id {} (captured from {})",
            self.width, self.height, self.bytes, self.id, self.path
        )
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}

pub(crate) fn data_url(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(22 + bytes.len().div_ceil(3) * 4);
    out.push_str("data:image/png;base64,");
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(a >> 2) as usize] as char);
        out.push(TABLE[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Validate the PNG signature, required structure, dimensions and every chunk
/// checksum. This deliberately does not decode DEFLATE pixels or transform them.
pub(crate) fn inspect(bytes: &[u8]) -> Result<(u32, u32), Error> {
    if bytes.starts_with(&[0xff, 0xd8])
        || bytes.starts_with(b"GIF8")
        || bytes.starts_with(b"RIFF")
        || bytes.starts_with(b"BM")
    {
        return Err(Error::Unsupported);
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(Error::Invalid);
    }
    let mut offset = 8;
    let mut dimensions = None;
    let mut idat = false;
    let mut idat_closed = false;
    let mut zlib_header = Vec::with_capacity(2);
    let mut ended = false;
    let mut indexed = false;
    let mut palette = false;
    while offset < bytes.len() {
        let header = bytes.get(offset..offset + 8).ok_or(Error::Invalid)?;
        let length =
            u32::from_be_bytes(header[..4].try_into().map_err(|_| Error::Invalid)?) as usize;
        let kind = &header[4..8];
        if !kind.iter().all(u8::is_ascii_alphabetic) || !kind[2].is_ascii_uppercase() {
            return Err(Error::Invalid);
        }
        let payload = bytes
            .get(offset + 8..offset + 8 + length)
            .ok_or(Error::Invalid)?;
        let expected = bytes
            .get(offset + 8 + length..offset + 12 + length)
            .ok_or(Error::Invalid)?;
        if crc32(&bytes[offset + 4..offset + 8 + length])
            != u32::from_be_bytes(expected.try_into().map_err(|_| Error::Invalid)?)
        {
            return Err(Error::Invalid);
        }
        if dimensions.is_none() && (kind != b"IHDR" || length != 13) {
            return Err(Error::Invalid);
        }
        match kind {
            b"IHDR" if dimensions.is_none() => {
                let width =
                    u32::from_be_bytes(payload[..4].try_into().map_err(|_| Error::Invalid)?);
                let height =
                    u32::from_be_bytes(payload[4..8].try_into().map_err(|_| Error::Invalid)?);
                let depth = payload[8];
                let color = payload[9];
                if width == 0
                    || height == 0
                    || payload[10] != 0
                    || payload[11] != 0
                    || payload[12] > 1
                    || !matches!(
                        (color, depth),
                        (0, 1 | 2 | 4 | 8 | 16)
                            | (2, 8 | 16)
                            | (3, 1 | 2 | 4 | 8)
                            | (4 | 6, 8 | 16)
                    )
                {
                    return Err(Error::Invalid);
                }
                indexed = color == 3;
                dimensions = Some((width, height));
            }
            b"PLTE"
                if !idat && !palette && (3..=768).contains(&length) && length.is_multiple_of(3) =>
            {
                palette = true
            }
            b"IDAT" if !payload.is_empty() && (!indexed || palette) && !idat_closed => {
                idat = true;
                let take = (2 - zlib_header.len()).min(payload.len());
                zlib_header.extend_from_slice(&payload[..take]);
            }
            b"IEND" if idat && length == 0 => {
                ended = true;
                offset += 12;
                break;
            }
            b"acTL" | b"fcTL" | b"fdAT" => return Err(Error::Unsupported),
            b"IHDR" | b"PLTE" | b"IDAT" | b"IEND" => return Err(Error::Invalid),
            _ if kind[0].is_ascii_uppercase() => return Err(Error::Unsupported),
            _ => {
                if idat {
                    idat_closed = true;
                }
            }
        }
        offset = offset.checked_add(12 + length).ok_or(Error::Invalid)?;
    }
    if !ended
        || offset != bytes.len()
        || zlib_header.len() != 2
        || zlib_header[0] & 15 != 8
        || zlib_header[0] >> 4 > 7
        || (u16::from(zlib_header[0]) * 256 + u16::from(zlib_header[1])) % 31 != 0
        || zlib_header[1] & 0x20 != 0
    {
        return Err(Error::Invalid);
    }
    dimensions.ok_or(Error::Invalid)
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb8_8320
            };
        }
    }
    !crc
}

#[cfg(test)]
pub(crate) fn fixture_png(color: [u8; 4]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        let start = out.len() - payload.len() - 4;
        out.extend_from_slice(&crc32(&out[start..]).to_be_bytes());
    }
    let raw = [0, color[0], color[1], color[2], color[3]];
    let mut b = 0u32;
    let mut sum = 1u32;
    for byte in raw {
        sum = (sum + u32::from(byte)) % 65521;
        b = (b + sum) % 65521;
    }
    let mut zlib = vec![0x78, 0x01, 0x01, 5, 0, 0xfa, 0xff];
    zlib.extend_from_slice(&raw);
    zlib.extend_from_slice(&((b << 16) | sum).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut png, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
    chunk(&mut png, b"IDAT", &zlib);
    chunk(&mut png, b"IEND", &[]);
    png
}

#[cfg(test)]
pub(crate) fn fixture_png_padded(color: [u8; 4], padding: usize) -> Vec<u8> {
    let png = fixture_png(color);
    let mut payload = b"Comment\0".to_vec();
    payload.extend(std::iter::repeat_n(b'x', padding));
    let mut out = png[..33].to_vec();
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(b"tEXt");
    out.extend_from_slice(&payload);
    out.extend_from_slice(&crc32(&out[33 + 4..]).to_be_bytes());
    out.extend_from_slice(&png[33..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_png_content_and_rejects_other_formats_or_broken_chunks() {
        let png = fixture_png([13, 29, 47, 255]);
        assert_eq!(inspect(&png), Ok((1, 1)));
        assert_eq!(data_url(&png)[..22].to_owned(), "data:image/png;base64,");
        let mut invalid = png.clone();
        invalid[45] ^= 1;
        assert_eq!(inspect(&invalid), Err(Error::Invalid));
        assert_eq!(inspect(&png[..png.len() - 1]), Err(Error::Invalid));
        assert_eq!(inspect(b"not an image"), Err(Error::Invalid));
        assert_eq!(inspect(&[0xff, 0xd8, 0xff, 0xd9]), Err(Error::Unsupported));
    }
}
