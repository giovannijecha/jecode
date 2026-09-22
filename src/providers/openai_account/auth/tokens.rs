use super::{Error, parse, required};
use crate::json::Value;
use std::fmt;

/// Claims are metadata from an authenticated token endpoint, not JWT signature validation.
pub struct Tokens {
    pub(super) access: String,
    pub(super) refresh: String,
    pub(super) account_id: String,
    pub(super) expires_at: u64,
}
impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Tokens { [redacted] }")
    }
}
impl Tokens {
    pub fn access_token(&self) -> &str {
        &self.access
    }
    pub fn refresh_token(&self) -> &str {
        &self.refresh
    }
    pub fn account_id(&self) -> &str {
        &self.account_id
    }
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
    pub(super) fn parse(body: &str, now: u64) -> Result<Self, Error> {
        let value = parse(body)?;
        Self::from_reply(&value, now, None)
    }
    pub(super) fn from_reply(
        value: &Value,
        now: u64,
        previous: Option<&Self>,
    ) -> Result<Self, Error> {
        if value
            .get("token_type")
            .is_some_and(|v| !v.text().is_some_and(|s| s.eq_ignore_ascii_case("Bearer")))
        {
            return Err(Error::Invalid);
        }
        let access = required(value, "access_token", 8192)?;
        let refresh = if value.get("refresh_token").is_none() {
            previous.ok_or(Error::Invalid)?.refresh_token()
        } else {
            required(value, "refresh_token", 8192)?
        };
        let lifetime = value
            .get("expires_in")
            .and_then(Value::unsigned)
            .filter(|n| *n > 0)
            .ok_or(Error::Invalid)?;
        let expires_at = now.checked_add(lifetime).ok_or(Error::Invalid)?;
        let claims = claims(access)?;
        let auth = claims
            .get("https://api.openai.com/auth")
            .ok_or(Error::Invalid)?;
        let account_id = required(auth, "chatgpt_account_id", 256)?.to_owned();
        if previous.is_some_and(|old| old.account_id != account_id) {
            return Err(Error::Invalid);
        }
        Ok(Self {
            access: access.to_owned(),
            refresh: refresh.to_owned(),
            account_id,
            expires_at,
        })
    }
}

pub(super) fn claims(token: &str) -> Result<Value, Error> {
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3
        || parts.iter().any(|p| {
            p.is_empty()
                || !p
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        })
    {
        return Err(Error::Invalid);
    }
    let input = parts[1].as_bytes();
    if input.len() % 4 == 1 {
        return Err(Error::Invalid);
    }
    let mut bytes = Vec::with_capacity(input.len() * 3 / 4);
    let (mut bits, mut count) = (0u32, 0usize);
    for &byte in input {
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return Err(Error::Invalid),
        };
        bits = (bits << 6) | u32::from(digit);
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    if bits != 0 {
        return Err(Error::Invalid);
    }
    parse(std::str::from_utf8(&bytes).map_err(|_| Error::Invalid)?)
}
