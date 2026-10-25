use super::{
    SignInUpdate, jwt, native,
    storage::{self, Record},
    wire,
};
use crate::cancel::Cancellation;
use crate::json::Value;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

mod page;
use page::respond;

pub(super) fn sign_in(
    host_id: &str,
    existing: Option<&Record>,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(SignInUpdate) -> Result<(), String>,
) -> Result<Record, String> {
    let mut registered = existing.map(|record| record.client_id.clone());
    for attempt in 0..2 {
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|_| "Could not start OpenAI sign-in callback listener")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "Could not configure OpenAI sign-in callback listener")?;
        let redirect = format!(
            "http://127.0.0.1:{}/auth/callback",
            listener
                .local_addr()
                .map_err(|_| "Could not read callback address")?
                .port()
        );
        let state = wire::base64url(&native::random(32, cancel)?);
        let nonce = wire::base64url(&native::random(32, cancel)?);
        let verifier = wire::base64url(&native::random(48, cancel)?);
        let challenge = wire::base64url(&native::sha256(verifier.as_bytes(), cancel)?);
        let client = registered.as_deref().unwrap_or("dynamic_agent_client");
        let mut fields = vec![
            ("client_id", client),
            ("ext_agent_host_id", host_id),
            ("response_type", "code"),
            ("redirect_uri", redirect.as_str()),
            ("scope", wire::SCOPES),
            ("resource", wire::RESOURCE),
            ("state", state.as_str()),
            ("nonce", nonce.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge.as_str()),
        ];
        if registered.is_none() {
            fields.push(("agent_name_hint", "Jecode"));
        }
        if existing.is_some_and(|record| {
            !record
                .scopes
                .iter()
                .any(|scope| scope == "chatgpt.tokens.use.direct")
        }) {
            fields.push(("prompt", "consent"));
        }
        if let Some(record) = existing
            && !record.email.is_empty()
        {
            fields.push(("login_hint", &record.email));
        }
        let url = format!("{}?{}", wire::AUTHORIZE, wire::form(&fields));
        emit(SignInUpdate::Browser(url.clone()))?;
        // The URL was already delivered for manual use; keep the listener alive
        // when a desktop opener is unavailable (headless and minimal systems).
        let _ = open_browser(&url);
        emit(SignInUpdate::Waiting)?;
        let callback = await_callback(&listener, &state, cancel)?;
        let issued = match &registered {
            Some(client_id) => {
                if callback.get("client_id").is_some_and(|id| id != client_id) {
                    return Err("Returning OpenAI client ID mismatch".into());
                }
                client_id.clone()
            }
            None => {
                let issued = callback
                    .get("client_id")
                    .ok_or("OpenAI registration returned no issued client ID")?;
                if issued == "dynamic_agent_client" || issued.is_empty() {
                    return Err("OpenAI registration returned an invalid issued client ID".into());
                }
                issued.clone()
            }
        };
        let code = callback
            .get("code")
            .filter(|code| !code.is_empty())
            .ok_or("OpenAI callback returned no code")?;
        let response = match wire::post(
            wire::TOKEN,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", &issued),
                ("code", code),
                ("code_verifier", &verifier),
                ("redirect_uri", &redirect),
                ("resource", wire::RESOURCE),
            ],
            cancel,
        ) {
            Ok(value) => value,
            Err(error) if attempt == 0 && wire::machine_code(&error) == Some("invalid_grant") => {
                registered = Some(issued);
                continue;
            }
            Err(error) => return Err(error),
        };
        let id_token = required(&response, "id_token")?;
        let claims = jwt::validate(id_token, &issued, &nonce, cancel)?;
        if existing.is_some_and(|record| record.subject != claims.subject) {
            return Err("Returning OpenAI account identity mismatch".into());
        }
        let access_token = required(&response, "access_token")?.to_string();
        if response
            .get("token_type")
            .and_then(Value::as_str)
            .is_none_or(|kind| !kind.eq_ignore_ascii_case("Bearer"))
        {
            return Err("OpenAI returned an unsupported token type".into());
        }
        let scope = required(&response, "scope")?;
        let scopes: Vec<String> = scope.split_whitespace().map(str::to_string).collect();
        let refresh_token = response
            .get("refresh_token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if scopes.iter().any(|scope| scope == "offline_access")
            && scopes
                .iter()
                .any(|scope| scope == "chatgpt.tokens.use.direct")
            && refresh_token.is_empty()
        {
            return Err("OpenAI token response omitted the granted refresh token".into());
        }
        let expires_in = seconds(&response, "expires_in")?;
        let now = storage::now()?;
        let earliest_refresh_at = earliest_refresh(&response, now)?;
        return Ok(Record {
            client_id: issued,
            subject: claims.subject,
            email: claims.email,
            label: claims.label,
            host_id: host_id.into(),
            id_token: id_token.into(),
            access_token,
            refresh_token,
            scopes,
            expires_at: now.saturating_add(expires_in),
            earliest_refresh_at,
        });
    }
    Err("OpenAI authorization code could not be exchanged; start sign-in again".into())
}

pub(super) fn refresh(record: &Record, cancel: &Cancellation) -> Result<Record, String> {
    if record.refresh_token.is_empty() {
        return Err("OpenAI account has no refresh token; sign in again".into());
    }
    let response = wire::post(
        wire::TOKEN,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &record.client_id),
            ("refresh_token", &record.refresh_token),
            ("resource", wire::RESOURCE),
        ],
        cancel,
    )?;
    let mut updated = record.clone();
    updated.access_token = required(&response, "access_token")?.to_string();
    updated.refresh_token = required(&response, "refresh_token")?.to_string();
    if let Some(id) = response.get("id_token").and_then(Value::as_str) {
        // A refresh ID token has no per-attempt nonce; retain the previously
        // verified ID token for reauthorization instead of trusting this one.
        if id.is_empty() {
            return Err("OpenAI returned an empty ID token".into());
        }
    }
    if let Some(scope) = response.get("scope").and_then(Value::as_str) {
        updated.scopes = scope.split_whitespace().map(str::to_string).collect();
    }
    if response
        .get("token_type")
        .and_then(Value::as_str)
        .is_none_or(|kind| !kind.eq_ignore_ascii_case("Bearer"))
    {
        return Err("OpenAI returned an unsupported token type".into());
    }
    let now = storage::now()?;
    updated.expires_at = now.saturating_add(seconds(&response, "expires_in")?);
    updated.earliest_refresh_at = earliest_refresh(&response, now)?;
    Ok(updated)
}

pub(super) fn revoke(record: &Record, cancel: &Cancellation) -> Result<(), String> {
    if record.refresh_token.is_empty() {
        return Ok(());
    }
    let discovery = wire::discovery(cancel)?;
    let endpoint = discovery
        .get("revocation_endpoint")
        .and_then(Value::as_str)
        .ok_or("OpenAI discovery has no revocation endpoint")?;
    for attempt in 0..3 {
        match wire::post(
            endpoint,
            &[
                ("client_id", &record.client_id),
                ("token", &record.refresh_token),
                ("token_type_hint", "refresh_token"),
            ],
            cancel,
        ) {
            Ok(_) => return Ok(()),
            Err(error)
                if attempt < 2
                    && (error.contains("HTTP 5") || error.contains("Could not complete")) =>
            {
                cancel.wait(Duration::from_millis(150 * (attempt + 1)))?
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

fn earliest_refresh(response: &Value, now: u64) -> Result<u64, String> {
    // The token reference names this field but does not define its format.
    // An integer Unix timestamp is accepted; other forms fail closed.
    match response.get("earliest_refresh_at") {
        Some(Value::Number(raw)) => raw
            .parse()
            .map_err(|_| "OpenAI earliest_refresh_at is invalid".into()),
        Some(Value::Null) => Ok(now.saturating_add(5)),
        Some(_) => Err("OpenAI earliest_refresh_at has an unsupported format".into()),
        None => Ok(now.saturating_add(5)),
    }
}

fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("OpenAI token response has no valid {key}"))
}

fn seconds(value: &Value, key: &str) -> Result<u64, String> {
    match value.get(key) {
        Some(Value::Number(raw)) => raw
            .parse()
            .map_err(|_| format!("OpenAI token response has invalid {key}")),
        _ => Err(format!("OpenAI token response has no valid {key}")),
    }
}

fn await_callback(
    listener: &TcpListener,
    state: &str,
    cancel: &Cancellation,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        match listener.accept() {
            Ok((mut stream, address)) => {
                if !address.ip().is_loopback() {
                    continue;
                }
                if let Some(callback) = handle_callback(
                    &mut stream,
                    listener
                        .local_addr()
                        .map_err(|_| "Could not read callback address")?
                        .port(),
                    state,
                    cancel,
                )? {
                    return Ok(callback);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                cancel.wait(Duration::from_millis(50))?
            }
            Err(_) => return Err("OpenAI callback listener failed".into()),
        }
    }
    Err("OpenAI sign-in timed out".into())
}

fn handle_callback(
    stream: &mut TcpStream,
    port: u16,
    state: &str,
    cancel: &Cancellation,
) -> Result<Option<std::collections::BTreeMap<String, String>>, String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|_| "Could not configure callback connection")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| "Could not configure callback connection")?;
    let mut input = Vec::with_capacity(1024);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !input.windows(4).any(|part| part == b"\r\n\r\n") {
        if cancel.requested() {
            return Err("Operation cancelled".into());
        }
        if Instant::now() >= deadline || input.len() >= 8192 {
            respond(stream, "400 Bad Request", "Invalid callback");
            return Ok(None);
        }
        let mut chunk = [0u8; 1024];
        match stream.read(&mut chunk) {
            Ok(0) => {
                respond(stream, "400 Bad Request", "Invalid callback");
                return Ok(None);
            }
            Ok(size) => input.extend_from_slice(&chunk[..size]),
            Err(error)
                if error.kind() == std::io::ErrorKind::TimedOut
                    || error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                continue;
            }
            Err(_) => {
                respond(stream, "400 Bad Request", "Invalid callback");
                return Ok(None);
            }
        }
    }
    let request = std::str::from_utf8(&input).map_err(|_| "OpenAI callback is not UTF-8")?;
    let Some((head, _)) = request.split_once("\r\n") else {
        respond(stream, "400 Bad Request", "Invalid callback");
        return Ok(None);
    };
    let Some(target) = head
        .strip_prefix("GET ")
        .and_then(|rest| rest.strip_suffix(" HTTP/1.1"))
    else {
        respond(stream, "405 Method Not Allowed", "Invalid callback");
        return Ok(None);
    };
    let Some(query) = target.strip_prefix("/auth/callback?") else {
        respond(stream, "404 Not Found", "Unknown path");
        return Ok(None);
    };
    let host = format!("Host: 127.0.0.1:{port}");
    if !request
        .split("\r\n")
        .any(|line| line.eq_ignore_ascii_case(&host))
    {
        respond(stream, "400 Bad Request", "Invalid callback host");
        return Ok(None);
    }
    let values = wire::query(query)?;
    if values.get("state").map(String::as_str) != Some(state) {
        respond(stream, "400 Bad Request", "Sign-in state mismatch");
        return Err("OpenAI callback state mismatch".into());
    }
    if let Some(error) = values.get("error") {
        respond(
            stream,
            "200 OK",
            "Sign-in was not completed. Return to Jecode.",
        );
        return Err(format!(
            "OpenAI authorization was declined: {}",
            if error
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                error.as_str()
            } else {
                "authorization_error"
            }
        ));
    }
    if !values.contains_key("code") {
        respond(stream, "400 Bad Request", "Missing authorization code");
        return Err("OpenAI callback returned no code".into());
    }
    respond(
        stream,
        "200 OK",
        "Return to Jecode. This tab can be closed.",
    );
    Ok(Some(values))
}

fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    let mut command = {
        let mut c = Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler").arg(url);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    crate::auth::isolate_environment(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
        .spawn()
        .map_err(|_| "Could not open the system browser; use the displayed sign-in URL")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn callback_accepts_fragmented_headers_and_rejects_wrong_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let receiver = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            handle_callback(&mut stream, port, "expected", &Cancellation::default())
        });
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .write_all(b"GET /auth/callback?code=secret&state=expected HTTP/1.1\r\nHo")
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        client
            .write_all(format!("st: 127.0.0.1:{port}\r\n\r\n").as_bytes())
            .unwrap();
        assert_eq!(receiver.join().unwrap().unwrap().unwrap()["code"], "secret");

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let receiver = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            handle_callback(&mut stream, port, "expected", &Cancellation::default())
        });
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.write_all(format!("GET /auth/callback?code=secret&state=wrong HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()).unwrap();
        assert!(
            receiver
                .join()
                .unwrap()
                .unwrap_err()
                .contains("state mismatch")
        );
    }

    #[test]
    fn unsupported_earliest_refresh_format_fails_closed() {
        let response = Value::object([("earliest_refresh_at", Value::string("tomorrow"))]);
        assert!(earliest_refresh(&response, 1).is_err());
        let response = Value::object([("earliest_refresh_at", Value::number(42))]);
        assert_eq!(earliest_refresh(&response, 1).unwrap(), 42);
    }
}
