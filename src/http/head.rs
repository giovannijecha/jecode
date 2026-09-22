use super::Error;

#[derive(Debug)]
pub struct Head {
    pub status: u16,
    fields: Vec<(String, String)>,
}
#[derive(Clone, Copy)]
pub(super) enum Body {
    Length(usize),
    Chunked,
    Close,
}

impl Head {
    /// Case-insensitive lookup for fields whose repetition was rejected during parsing.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
    pub fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.fields
            .iter()
            .filter(move |(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub(super) fn parse(bytes: &[u8]) -> Result<(Head, Body), Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Invalid)?;
    let mut lines = text.split("\r\n");
    let status_line = lines.next().ok_or(Error::Invalid)?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().ok_or(Error::Invalid)?;
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(Error::Unsupported);
    }
    let code = parts.next().ok_or(Error::Invalid)?;
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Invalid);
    }
    let status = code.parse::<u16>().map_err(|_| Error::Invalid)?;
    if !(100..=599).contains(&status) || !valid_value(parts.next().ok_or(Error::Invalid)?) {
        return Err(Error::Invalid);
    }
    if status == 101 {
        return Err(Error::Unsupported);
    }
    let mut head = Head {
        status,
        fields: Vec::new(),
    };
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if head.fields.len() >= 128 {
            return Err(Error::Limit);
        }
        let (key, value) = field(line)?;
        if matches!(
            key.as_str(),
            "content-length" | "transfer-encoding" | "content-type" | "content-encoding"
        ) && head.get(&key).is_some()
        {
            return Err(Error::AmbiguousLength);
        }
        head.fields.push((key, value));
    }
    let length = head
        .get("content-length")
        .map(|value| {
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Error::Invalid);
            }
            value.parse::<usize>().map_err(|_| Error::Limit)
        })
        .transpose()?;
    let encoding = head.get("transfer-encoding");
    if version == "HTTP/1.0" && encoding.is_some() {
        return Err(Error::Unsupported);
    }
    if encoding.is_some() && length.is_some() {
        return Err(Error::AmbiguousLength);
    }
    if head
        .get("content-encoding")
        .is_some_and(|s| !s.eq_ignore_ascii_case("identity"))
    {
        return Err(Error::Unsupported);
    }
    if status < 200 || status == 204 {
        if length.is_some() || encoding.is_some() {
            return Err(Error::Invalid);
        }
        return Ok((head, Body::Length(0)));
    }
    if status == 304 {
        return Ok((head, Body::Length(0)));
    }
    let body = match (encoding, length) {
        (Some(value), _) if version == "HTTP/1.1" && value.eq_ignore_ascii_case("chunked") => {
            Body::Chunked
        }
        (Some(_), _) => return Err(Error::Unsupported),
        (_, Some(length)) => Body::Length(length),
        _ => Body::Close,
    };
    Ok((head, body))
}

pub(super) fn field(line: &str) -> Result<(String, String), Error> {
    let (name, value) = line.split_once(':').ok_or(Error::Invalid)?;
    if name.is_empty() || !name.bytes().all(super::token) || !valid_value(value) {
        return Err(Error::Invalid);
    }
    Ok((
        name.to_ascii_lowercase(),
        value.trim_matches([' ', '\t']).to_owned(),
    ))
}

pub(super) fn valid_value(value: &str) -> bool {
    value.bytes().all(|b| b == b'\t' || (32..=126).contains(&b))
}
