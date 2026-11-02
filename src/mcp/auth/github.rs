use super::super::types::{AuthMethod, Connection, Error, ErrorKind};
use super::{TokenContext, Update, token_record, wire};
use crate::cancel::Cancellation;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEVICE: &str = "https://github.com/login/device/code";
const TOKEN: &str = "https://github.com/login/oauth/access_token";
const ISSUER: &str = "https://github.com";
// Public identifier of the maintainer-owned App, shared by every Jecode user.
const CLIENT_ID: &str = "Iv23liTdak2Sf8VpPtLw";

/// An explicit override, then the maintainer variable, then the bundled App.
fn client_id(connection: &Connection) -> String {
    let valid =
        |s: &String| !s.is_empty() && s.len() <= 2048 && s.bytes().all(|b| b.is_ascii_graphic());
    connection
        .oauth_client_id
        .clone()
        .or_else(|| std::env::var("JECODE_GITHUB_CLIENT_ID").ok().filter(valid))
        .or_else(|| {
            option_env!("JECODE_GITHUB_CLIENT_ID")
                .map(str::to_owned)
                .filter(valid)
        })
        .unwrap_or_else(|| CLIENT_ID.to_owned())
}
pub(super) fn eligible(connection: &Connection) -> bool {
    super::super::config::github_endpoint(&connection.endpoint)
}
pub(super) fn sign_in(
    mut connection: Connection,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(Update) -> Result<(), String>,
) -> Result<Connection, Error> {
    if !eligible(&connection) {
        return Err(wire::failure(
            "GitHub device flow requires the GitHub MCP endpoint",
        ));
    }
    super::super::config::validate(&connection)?;
    // GitHub App permissions, not OAuth scopes, bound the token.
    let id = client_id(&connection);
    let value = wire::post_form(DEVICE, &[("client_id", &id)], cancel)?;
    let device = wire::required(&value, "device_code")?;
    let code = wire::required(&value, "user_code")?;
    let url = wire::required(&value, "verification_uri")?;
    if url != "https://github.com/login/device" {
        return Err(wire::failure(
            "GitHub returned an unexpected verification URL",
        ));
    }
    let lifetime = wire::seconds(&value, "expires_in").unwrap_or(900).min(1800);
    let mut interval = wire::seconds(&value, "interval").unwrap_or(5).max(5);
    emit(Update::DeviceCode {
        url: url.into(),
        code: code.into(),
    })
    .map_err(|_| wire::failure("GitHub sign-in was cancelled"))?;
    #[cfg(not(test))]
    let _ = crate::auth::openai::open_browser(url);
    emit(Update::Waiting).map_err(|_| wire::failure("GitHub sign-in was cancelled"))?;
    let deadline = Instant::now() + Duration::from_secs(lifetime);
    loop {
        if Instant::now() >= deadline {
            return Err(wire::failure("GitHub device code expired"));
        }
        cancel
            .wait(Duration::from_secs(interval))
            .map_err(|_| Error::new(ErrorKind::Cancelled, "Operation cancelled"))?;
        let value = wire::post_form(
            TOKEN,
            &[
                ("client_id", &id),
                ("device_code", device),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ],
            cancel,
        )?;
        match value.get("error").and_then(crate::json::Value::as_str) {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval = wire::seconds(&value, "interval")
                    .unwrap_or(interval + 5)
                    .max(interval + 5);
                continue;
            }
            Some(_) => return Err(wire::failure("GitHub device authorization failed")),
            None => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| wire::failure("System clock is invalid"))?
                    .as_secs();
                connection.oauth = Some(token_record(
                    &value,
                    TokenContext {
                        endpoint: TOKEN,
                        bound_endpoint: &connection.endpoint,
                        client: &id,
                        issuer: ISSUER,
                        resource: Some(&connection.endpoint),
                        github: true,
                        now,
                    },
                    None,
                )?);
                connection.method = AuthMethod::GitHubDevice;
                connection.token.clear();
                super::super::config::validate(&connection)?;
                return Ok(connection);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_matches_with_or_without_trailing_slash_and_keeps_overrides_explicit() {
        for (url, expected) in [
            ("https://api.githubcopilot.com/mcp/", true),
            ("https://api.githubcopilot.com/mcp", true),
            ("https://api.githubcopilot.com/mcp/readonly", true),
            ("https://api.githubcopilot.com/mcpx", false),
            ("https://api.githubcopilot.com.evil.test/mcp", false),
        ] {
            assert_eq!(
                eligible(&Connection::draft("GitHub", url)),
                expected,
                "{url}"
            );
        }
        let mut connection = Connection::draft("GitHub", "https://api.githubcopilot.com/mcp");
        if std::env::var_os("JECODE_GITHUB_CLIENT_ID").is_none()
            && option_env!("JECODE_GITHUB_CLIENT_ID").is_none()
        {
            assert_eq!(client_id(&connection), CLIENT_ID);
        }
        connection.oauth_client_id = Some("Iv1.override".into());
        assert_eq!(client_id(&connection), "Iv1.override");
    }

    #[test]
    fn invalid_explicit_github_client_id_never_falls_back_to_another_app() {
        let mut connection = Connection::draft("GitHub", "https://api.githubcopilot.com/mcp");
        connection.method = AuthMethod::GitHubDevice;
        connection.oauth_client_id = Some("Iv1.é".into());
        assert!(super::super::super::config::validate(&connection).is_err());
        assert_eq!(client_id(&connection), "Iv1.é");
    }
}
