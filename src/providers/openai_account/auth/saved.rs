//! Plain JSON representation, separate from provider replies and canonical history.
use super::{Error, Tokens, parse, required, text, tokens::claims};
use crate::json::{self, Value};

impl Tokens {
    pub fn saved_json(&self) -> Result<String, Error> {
        let mut fields = vec![
            ("version", Value::Number("1".into())),
            ("state", text("ready")),
            ("provider", text("openai-account")),
            ("access_token", text(&self.access)),
            ("refresh_token", text(&self.refresh)),
            ("account_id", text(&self.account_id)),
            ("expires_at", Value::Number(self.expires_at.to_string())),
        ];
        if let Some(generation) = &self.generation {
            fields.push(("generation", text(generation)));
        }
        json::encode(&json::object(fields), 32768).map_err(|_| Error::Invalid)
    }

    pub fn from_saved_json(body: &str) -> Result<Option<Self>, Error> {
        let value = parse(body)?;
        if value.get("version").and_then(Value::unsigned) != Some(1) {
            return Err(Error::Invalid);
        }
        match value.get("state").and_then(Value::text) {
            Some("signed_out" | "reauth_required") => return Ok(None),
            Some("ready") => {}
            _ => return Err(Error::Invalid),
        }
        if value.get("provider").and_then(Value::text) != Some("openai-account") {
            return Err(Error::Invalid);
        }
        let access = required(&value, "access_token", 8192)?;
        let account_id = required(&value, "account_id", 256)?;
        let claims = claims(access)?;
        if claims
            .get("https://api.openai.com/auth")
            .and_then(|v| v.get("chatgpt_account_id"))
            .and_then(Value::text)
            != Some(account_id)
        {
            return Err(Error::Invalid);
        }
        Ok(Some(Self {
            access: access.into(),
            refresh: required(&value, "refresh_token", 8192)?.into(),
            account_id: account_id.into(),
            expires_at: value
                .get("expires_at")
                .and_then(Value::unsigned)
                .filter(|n| *n > 0)
                .ok_or(Error::Invalid)?,
            generation: match value.get("generation") {
                None => None,
                Some(Value::String(generation))
                    if generation.len() <= 80
                        && generation
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() || b == b'-') =>
                {
                    Some(generation.clone())
                }
                _ => return Err(Error::Invalid),
            },
        }))
    }
}
