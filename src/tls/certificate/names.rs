use super::Error;
pub(super) fn dns(value: &[u8], wildcard: bool) -> Result<&str, Error> {
    let name = std::str::from_utf8(value).map_err(|_| Error::Name)?;
    let ordinary = if wildcard {
        name.strip_prefix("*.").unwrap_or(name)
    } else {
        name
    };
    if name.len() > 253
        || ordinary.parse::<std::net::IpAddr>().is_ok()
        || ordinary.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err(Error::Name);
    }
    if ordinary != name && !ordinary.contains('.') {
        return Err(Error::Name);
    }
    Ok(name)
}
pub(super) fn matches(pattern: &str, host: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host.split_once('.')
            .is_some_and(|(first, rest)| !first.is_empty() && rest.eq_ignore_ascii_case(suffix))
    } else {
        pattern.eq_ignore_ascii_case(host)
    }
}
