use super::super::types::{AuthMethod, Connection, Error, ErrorKind};
use super::{TokenContext, Update, token_record, wire};
use crate::cancel::Cancellation;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEVICE: &str = "https://github.com/login/device/code";
const TOKEN: &str = "https://github.com/login/oauth/access_token";
const ISSUER: &str = "https://github.com";

pub(super) fn client_id(connection: &Connection) -> Option<String> {
    connection
        .oauth_client_id
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("JECODE_GITHUB_CLIENT_ID")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            option_env!("JECODE_GITHUB_CLIENT_ID")
                .map(str::to_owned)
                .filter(|s| !s.is_empty())
        })
        .filter(|s| s.len() <= 2048 && s.bytes().all(|b| b.is_ascii_graphic()))
}
pub(super) fn eligible(connection: &Connection) -> bool {
    connection
        .endpoint
        .starts_with("https://api.githubcopilot.com/mcp/")
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
    let id = client_id(&connection)
        .ok_or_else(|| wire::failure("GitHub App client ID is not configured"))?;
    let value = wire::post_form(
        DEVICE,
        &[("client_id", &id), ("scope", "offline_access")],
        cancel,
    )?;
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
                connection.oauth_client_id = Some(id);
                connection.method = AuthMethod::GitHubDevice;
                connection.token.clear();
                super::super::config::validate(&connection)?;
                return Ok(connection);
            }
        }
    }
}
