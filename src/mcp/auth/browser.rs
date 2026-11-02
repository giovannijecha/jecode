use super::super::types::{Connection, Error, ErrorKind};
use super::{TokenContext, Update, discovery, token_record, wire};
use crate::{auth::openai, cancel::Cancellation};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) fn sign_in(
    mut connection: Connection,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(Update) -> Result<(), String>,
) -> Result<Connection, Error> {
    let challenge = super::probe(&connection, cancel)?
        .challenge
        .unwrap_or_default();
    let server = discovery::discover_challenged(&connection, cancel, &challenge)?;
    if connection.oauth.as_ref().is_some_and(|tokens| {
        tokens.issuer != server.issuer || tokens.endpoint != connection.endpoint
    }) {
        return Err(wire::failure(
            "MCP OAuth issuer or endpoint changed; configure a new connection",
        ));
    }
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|_| wire::failure("Could not start OAuth callback listener"))?;
    listener
        .set_nonblocking(true)
        .map_err(|_| wire::failure("Could not configure OAuth callback listener"))?;
    let port = listener
        .local_addr()
        .map_err(|_| wire::failure("Could not read OAuth callback port"))?
        .port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let client = discovery::client_id(&connection, &server, &redirect, cancel)?;
    let random = |n| {
        openai::native::random(n, cancel)
            .map_err(|_| wire::failure("OAuth random generator failed"))
    };
    let state = wire::base64url(&random(32)?);
    let verifier = wire::base64url(&random(48)?);
    let challenge = wire::base64url(
        &openai::native::sha256(verifier.as_bytes(), cancel)
            .map_err(|_| wire::failure("OAuth PKCE generation failed"))?,
    );
    let mut fields = vec![
        ("response_type", "code"),
        ("client_id", client.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("code_challenge_method", "S256"),
        ("code_challenge", challenge.as_str()),
        ("state", state.as_str()),
        ("resource", server.resource.as_str()),
    ];
    if let Some(scope) = &server.scope {
        fields.push(("scope", scope));
    }
    let separator = if server.authorize.contains('?') {
        '&'
    } else {
        '?'
    };
    let url = format!("{}{separator}{}", server.authorize, wire::form(&fields));
    emit(Update::Browser(url.clone())).map_err(|_| wire::failure("OAuth sign-in was cancelled"))?;
    #[cfg(not(test))]
    let _ = openai::open_browser(&url);
    emit(Update::Waiting).map_err(|_| wire::failure("OAuth sign-in was cancelled"))?;
    let code = wait(
        &listener,
        port,
        &state,
        &server.issuer,
        server.response_iss_required,
        cancel,
    )?;
    let value = wire::post_form(
        &server.token,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &client),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", &redirect),
            ("resource", &server.resource),
        ],
        cancel,
    )?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| wire::failure("System clock is invalid"))?
        .as_secs();
    connection.oauth = Some(token_record(
        &value,
        TokenContext {
            endpoint: &server.token,
            bound_endpoint: &connection.endpoint,
            client: &client,
            issuer: &server.issuer,
            resource: Some(&server.resource),
            github: false,
            now,
        },
        None,
    )?);
    connection.method = super::super::types::AuthMethod::Browser;
    connection.token.clear();
    super::super::config::validate(&connection)?;
    Ok(connection)
}

fn wait(
    listener: &TcpListener,
    port: u16,
    state: &str,
    issuer: &str,
    iss_required: bool,
    cancel: &Cancellation,
) -> Result<String, Error> {
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        if cancel.requested() {
            return Err(Error::new(ErrorKind::Cancelled, "Operation cancelled"));
        }
        match listener.accept() {
            Ok((mut stream, peer)) if peer.ip().is_loopback() => {
                // Windows sockets inherit non-blocking mode, which would void the read timeout.
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                if let Some(code) =
                    callback(&mut stream, port, state, issuer, iss_required, cancel)?
                {
                    return Ok(code);
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                cancel
                    .wait(Duration::from_millis(30))
                    .map_err(|_| Error::new(ErrorKind::Cancelled, "Operation cancelled"))?;
            }
            Err(_) => return Err(wire::failure("OAuth callback listener failed")),
        }
    }
    Err(wire::failure("OAuth sign-in timed out"))
}
fn callback(
    stream: &mut TcpStream,
    port: u16,
    state: &str,
    issuer: &str,
    iss_required: bool,
    cancel: &Cancellation,
) -> Result<Option<String>, Error> {
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|_| wire::failure("OAuth callback failed"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| wire::failure("OAuth callback failed"))?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut bytes = Vec::new();
    while !bytes.windows(4).any(|p| p == b"\r\n\r\n") {
        if cancel.requested() {
            return Err(Error::new(ErrorKind::Cancelled, "Operation cancelled"));
        }
        if bytes.len() >= 8192 || Instant::now() > deadline {
            respond(stream, false);
            return Ok(None);
        }
        let mut chunk = [0; 1024];
        match stream.read(&mut chunk) {
            Ok(0) => {
                respond(stream, false);
                return Ok(None);
            }
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                respond(stream, false);
                return Ok(None);
            }
        }
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        respond(stream, false);
        return Ok(None);
    };
    let Some((line, headers)) = text.split_once("\r\n") else {
        respond(stream, false);
        return Ok(None);
    };
    let Some(query) = line
        .strip_prefix("GET /callback?")
        .and_then(|s| s.strip_suffix(" HTTP/1.1"))
    else {
        respond(stream, false);
        return Ok(None);
    };
    if !headers
        .split("\r\n")
        .any(|h| h.eq_ignore_ascii_case(&format!("Host: 127.0.0.1:{port}")))
    {
        respond(stream, false);
        return Ok(None);
    }
    // Any local process can reach the listener: only a request carrying this
    // sign-in's state may end it.
    let Ok(values) = wire::query(query) else {
        respond(stream, false);
        return Ok(None);
    };
    if values.get("state").map(String::as_str) != Some(state) {
        respond(stream, false);
        return Ok(None);
    }
    if values.get("iss").is_some_and(|iss| iss != issuer)
        || (iss_required && !values.contains_key("iss"))
    {
        respond(stream, false);
        return Err(wire::failure("OAuth authorization issuer did not match"));
    }
    if values.contains_key("error") {
        respond(stream, true);
        return Err(wire::failure("OAuth authorization was declined"));
    }
    let Some(code) = values
        .get("code")
        .filter(|code| !code.is_empty() && code.len() <= 4096)
    else {
        respond(stream, false);
        return Ok(None);
    };
    respond(stream, true);
    Ok(Some(code.clone()))
}
fn respond(stream: &mut TcpStream, ok: bool) {
    let body = if ok {
        "Return to Jecode. This tab can be closed."
    } else {
        "Invalid sign-in callback."
    };
    let status = if ok { "200 OK" } else { "400 Bad Request" };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let request = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        let (head, tail) = request.split_at(12);
        stream.write_all(head.as_bytes()).unwrap();
        // Longer than the callback read timeout, which must hold on accepted sockets.
        std::thread::sleep(Duration::from_millis(250));
        stream.write_all(tail.as_bytes()).unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        reply
    }

    fn listen() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    #[test]
    fn stray_callbacks_are_ignored_until_the_matching_state_arrives() {
        let (listener, port) = listen();
        let client = std::thread::spawn(move || {
            for target in [
                "/callback?%zz",
                "/callback?flag",
                "/callback?state=other&iss=https%3A%2F%2Fevil.test&code=x",
            ] {
                assert!(send(port, target).starts_with("HTTP/1.1 400"), "{target}");
            }
            let good = "/callback?state=right&code=good&iss=https%3A%2F%2Fissuer.test";
            assert!(send(port, good).starts_with("HTTP/1.1 200"));
        });
        let cancel = Cancellation::default();
        let code = wait(
            &listener,
            port,
            "right",
            "https://issuer.test",
            true,
            &cancel,
        );
        assert_eq!(code.unwrap(), "good");
        client.join().unwrap();
    }

    #[test]
    fn matching_state_from_another_issuer_ends_sign_in() {
        let (listener, port) = listen();
        let client = std::thread::spawn(move || {
            send(
                port,
                "/callback?state=right&code=x&iss=https%3A%2F%2Fevil.test",
            );
        });
        let cancel = Cancellation::default();
        assert!(
            wait(
                &listener,
                port,
                "right",
                "https://issuer.test",
                false,
                &cancel
            )
            .is_err()
        );
        client.join().unwrap();
    }
}
