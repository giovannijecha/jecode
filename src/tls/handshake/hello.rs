//! Single-suite, non-PSK ClientHello and strict ServerHello negotiation.
use super::wire::{Cursor, extension, extensions, vector16};
use crate::tls::Error;

const RETRY_RANDOM: [u8; 32] = [
    0xcf, 0x21, 0xad, 0x74, 0xe5, 0x9a, 0x61, 0x11, 0xbe, 0x1d, 0x8c, 0x02, 0x1e, 0x65, 0xb8, 0x91,
    0xc2, 0xa2, 0x11, 0x16, 0x7a, 0xbb, 0x8c, 0x5e, 0x07, 0x9e, 0x09, 0xe2, 0xc8, 0xa8, 0x33, 0x9c,
];

pub(super) fn client(host: &str, random: &[u8; 32], public: &[u8; 32]) -> Result<Vec<u8>, Error> {
    if host.len() > 253
        || host.parse::<std::net::IpAddr>().is_ok()
        || host.split('.').any(|part| {
            part.is_empty()
                || part.len() > 63
                || part.starts_with('-')
                || part.ends_with('-')
                || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err(Error::Malformed);
    }
    let mut names = vec![0];
    vector16(&mut names, host.as_bytes());
    let mut server_name = Vec::new();
    vector16(&mut server_name, &names);
    let mut exts = Vec::new();
    extension(&mut exts, 0, &server_name);
    extension(&mut exts, 43, &[2, 3, 4]); // supported_versions: TLS 1.3 only
    extension(&mut exts, 10, &[0, 2, 0, 29]); // supported_groups: X25519 only
    extension(&mut exts, 13, &[0, 4, 8, 4, 4, 3]); // RSA-PSS/SHA256, ECDSA-P256/SHA256
    extension(&mut exts, 16, b"\x00\x09\x08http/1.1");
    let mut entry = vec![0, 29];
    vector16(&mut entry, public);
    let mut shares = Vec::new();
    vector16(&mut shares, &entry);
    extension(&mut exts, 51, &shares);
    let mut body = vec![3, 3];
    body.extend_from_slice(random);
    body.push(0); // Empty legacy session ID; no middlebox compatibility mode.
    vector16(&mut body, &[0x13, 1]);
    body.extend_from_slice(&[1, 0]);
    vector16(&mut body, &exts);
    let length = body.len();
    let mut message = vec![1, (length >> 16) as u8, (length >> 8) as u8, length as u8];
    message.extend_from_slice(&body);
    Ok(message)
}

pub(super) fn server(body: &[u8]) -> Result<[u8; 32], Error> {
    let mut input = Cursor(body);
    if input.word()? != 0x0303 {
        return Err(Error::Unsupported);
    }
    if input.take(32)? == RETRY_RANDOM {
        return Err(Error::Unsupported);
    }
    if !input.vector8()?.is_empty() {
        return Err(Error::Malformed);
    }
    if input.word()? != 0x1301 || input.byte()? != 0 {
        return Err(Error::Unsupported);
    }
    let exts = extensions(input.vector16()?)?;
    input.end()?;
    let mut version = false;
    let mut public = None;
    for (kind, bytes) in exts {
        match kind {
            43 if bytes == [3, 4] => version = true,
            51 => {
                let mut share = Cursor(bytes);
                if share.word()? != 29 {
                    return Err(Error::Unsupported);
                }
                public = Some(share.vector16()?.try_into().map_err(|_| Error::Malformed)?);
                share.end()?;
            }
            _ => return Err(Error::Unsupported),
        }
    }
    if !version {
        return Err(Error::Malformed);
    }
    public.ok_or(Error::Malformed)
}

pub(super) fn encrypted_extensions(body: &[u8]) -> Result<(), Error> {
    let mut input = Cursor(body);
    let exts = extensions(input.vector16()?)?;
    input.end()?;
    for (kind, bytes) in exts {
        match kind {
            0 if bytes.is_empty() => {}
            16 if bytes == b"\x00\x09\x08http/1.1" => {}
            10 => {
                let mut groups = Cursor(bytes);
                let list = groups.vector16()?;
                groups.end()?;
                if list.is_empty() || !list.len().is_multiple_of(2) {
                    return Err(Error::Malformed);
                }
            }
            _ => return Err(Error::Unsupported),
        }
    }
    Ok(())
}
