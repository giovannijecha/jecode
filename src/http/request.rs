use super::{Error, head};
use std::collections::BTreeSet;

/// Prepare one HTTP/1.1 JSON POST. Sending requires a separately authenticated TLS peer.
/// The returned bytes can contain credentials; never log or persist them.
pub fn post_json(
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, Error> {
    post(host, path, headers, body, max_bytes, "application/json")
}

pub fn post_form(
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, Error> {
    post(
        host,
        path,
        headers,
        body,
        max_bytes,
        "application/x-www-form-urlencoded",
    )
}

fn post(
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
    max_bytes: usize,
    content_type: &str,
) -> Result<Vec<u8>, Error> {
    if host.len() > 253
        || host.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err(Error::Invalid);
    }
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.len() > 8192
        || !path.bytes().all(|b| (33..=126).contains(&b) && b != b'#')
    {
        return Err(Error::Invalid);
    }
    if headers.len() > 64 {
        return Err(Error::Limit);
    }
    let mut seen = BTreeSet::new();
    for (name, value) in headers {
        let name = name.to_ascii_lowercase();
        if name.is_empty()
            || !name.bytes().all(super::token)
            || !head::valid_value(value)
            || !seen.insert(name.clone())
            || matches!(
                name.as_str(),
                "host"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
                    | "content-type"
                    | "accept-encoding"
                    | "expect"
                    | "upgrade"
                    | "trailer"
                    | "te"
            )
        {
            return Err(Error::Invalid);
        }
    }
    let mut bytes = Vec::new();
    append(
        &mut bytes,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: {content_type}\r\nAccept-Encoding: identity\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        ),
        max_bytes,
    )?;
    for (name, value) in headers {
        append(&mut bytes, name, max_bytes)?;
        append(&mut bytes, ": ", max_bytes)?;
        append(&mut bytes, value, max_bytes)?;
        append(&mut bytes, "\r\n", max_bytes)?;
    }
    if bytes.len() > 32766 {
        return Err(Error::Limit);
    }
    append(&mut bytes, "\r\n", max_bytes)?;
    append(&mut bytes, body, max_bytes)?;
    Ok(bytes)
}
fn append(output: &mut Vec<u8>, value: &str, max: usize) -> Result<(), Error> {
    if value.len() > max.saturating_sub(output.len()) {
        return Err(Error::Limit);
    }
    output.extend_from_slice(value.as_bytes());
    Ok(())
}
