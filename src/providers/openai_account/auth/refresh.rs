use super::{AUTH_HOST, CLIENT_ID, Error, Tokens, form, parse};
use crate::{http, json::Value};

impl Tokens {
    pub fn refresh_request(&self) -> Result<Vec<u8>, Error> {
        let body = form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", self.refresh_token()),
        ]);
        http::post_form(
            AUTH_HOST,
            "/oauth/token",
            &[("Accept", "application/json")],
            &body,
            65536,
        )
        .map_err(|_| Error::Invalid)
    }

    /// Refresh tokens may rotate. An omitted replacement retains the previous token.
    pub fn refreshed(&self, status: u16, body: &str, now: u64) -> Result<Self, Error> {
        if status != 200 {
            if matches!(status, 400 | 401)
                && parse(body)?.get("error").and_then(Value::text) == Some("invalid_grant")
            {
                return Err(Error::Denied);
            }
            return Err(Error::Remote);
        }
        Self::from_reply(&parse(body)?, now, Some(self))
    }
}
