use super::super::types::{Connection, Error, ErrorKind};
use super::{TokenContext, Update, client, discovery, scopes, token_record, wire};
use crate::{
    auth::{
        landing::{self, Landing},
        openai,
    },
    cancel::Cancellation,
};
use std::{
    io::Read,
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
    let client = client::resolve(&connection, &server, &redirect, cancel)?;
    let random =
        |n| openai::random(n, cancel).map_err(|_| wire::failure("OAuth random generator failed"));
    let state = wire::base64url(&random(32)?);
    let verifier = wire::base64url(&random(48)?);
    let challenge = wire::base64url(
        &openai::sha256(verifier.as_bytes(), cancel)
            .map_err(|_| wire::failure("OAuth PKCE generation failed"))?,
    );
    let mut fields = vec![
        ("response_type", "code"),
        ("client_id", client.id.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("code_challenge_method", "S256"),
        ("code_challenge", challenge.as_str()),
        ("state", state.as_str()),
        ("resource", server.resource.as_str()),
    ];
    let requested_scope = requested_scope(&connection, &server)?;
    if let Some(scope) = &requested_scope {
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
    let (code, landing) = wait(
        &listener,
        port,
        &state,
        &server.issuer,
        server.response_iss_required,
        cancel,
    )?;
    let value = client::token_request(
        &server.token,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", &redirect),
            ("resource", &server.resource),
        ],
        &client,
        cancel,
    )?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| wire::failure("System clock is invalid"))?
        .as_secs();
    let mut tokens = token_record(
        &value,
        TokenContext {
            endpoint: &server.token,
            bound_endpoint: &connection.endpoint,
            client: &client.id,
            client_auth_method: client.method,
            client_secret: client.secret.as_deref(),
            client_secret_expires_at: client.secret_expires_at,
            issuer: &server.issuer,
            resource: Some(&server.resource),
            github: false,
            now,
        },
        None,
    )?;
    // OAuth permits omitting an unchanged scope in the token response.
    // Keep the authorized request so a later step-up can retain those grants.
    if tokens.scope.is_none() {
        tokens.scope = requested_scope;
    }
    connection.oauth = Some(tokens);
    connection.pending_scope = None;
    connection.method = super::super::types::AuthMethod::Browser;
    connection.token.clear();
    super::super::config::validate(&connection)?;
    // The connection saves later, after its own check, which reports in Jecode: what the
    // browser asked for has already succeeded.
    landing.authenticated();
    Ok(connection)
}

fn wait(
    listener: &TcpListener,
    port: u16,
    state: &str,
    issuer: &str,
    iss_required: bool,
    cancel: &Cancellation,
) -> Result<(String, Landing), Error> {
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        if cancel.requested() {
            return Err(Error::new(ErrorKind::Cancelled, "Operation cancelled"));
        }
        match listener.accept() {
            Ok((stream, peer)) if peer.ip().is_loopback() => {
                // Windows sockets inherit non-blocking mode, which would void the read timeout.
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                if let Some(found) = callback(stream, port, state, issuer, iss_required, cancel)? {
                    return Ok(found);
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
    mut stream: TcpStream,
    port: u16,
    state: &str,
    issuer: &str,
    iss_required: bool,
    cancel: &Cancellation,
) -> Result<Option<(String, Landing)>, Error> {
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
            landing::answer(&mut stream, "400 Bad Request", false);
            return Ok(None);
        }
        let mut chunk = [0; 1024];
        match stream.read(&mut chunk) {
            Ok(0) => {
                landing::answer(&mut stream, "400 Bad Request", false);
                return Ok(None);
            }
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                landing::answer(&mut stream, "400 Bad Request", false);
                return Ok(None);
            }
        }
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        landing::answer(&mut stream, "400 Bad Request", false);
        return Ok(None);
    };
    let Some((line, headers)) = text.split_once("\r\n") else {
        landing::answer(&mut stream, "400 Bad Request", false);
        return Ok(None);
    };
    let Some(query) = line
        .strip_prefix("GET /callback?")
        .and_then(|s| s.strip_suffix(" HTTP/1.1"))
    else {
        landing::answer(&mut stream, "400 Bad Request", false);
        return Ok(None);
    };
    if !headers
        .split("\r\n")
        .any(|h| h.eq_ignore_ascii_case(&format!("Host: 127.0.0.1:{port}")))
    {
        landing::answer(&mut stream, "400 Bad Request", false);
        return Ok(None);
    }
    // Any local process can reach the listener: only a request carrying this
    // sign-in's state may end it.
    let Ok(values) = wire::query(query) else {
        landing::answer(&mut stream, "400 Bad Request", false);
        return Ok(None);
    };
    if values.get("state").map(String::as_str) != Some(state) {
        landing::answer(&mut stream, "400 Bad Request", false);
        return Ok(None);
    }
    if values.get("iss").is_some_and(|iss| iss != issuer)
        || (iss_required && !values.contains_key("iss"))
    {
        landing::answer(&mut stream, "400 Bad Request", false);
        return Err(wire::failure("OAuth authorization issuer did not match"));
    }
    if values.contains_key("error") {
        landing::answer(&mut stream, "200 OK", false);
        return Err(wire::failure("OAuth authorization was declined"));
    }
    let Some(code) = values
        .get("code")
        .filter(|code| !code.is_empty() && code.len() <= 4096)
    else {
        landing::answer(&mut stream, "400 Bad Request", false);
        return Ok(None);
    };
    let code = code.clone();
    Ok(Some((code, Landing::new(stream))))
}

/// Step-up scopes first, then scopes already granted by the same
/// authorization server, then the resource's own scopes.
fn requested_scope(
    connection: &Connection,
    server: &discovery::Server,
) -> Result<Option<String>, Error> {
    let scope = scopes::union(&[
        connection.pending_scope.as_deref(),
        connection
            .oauth
            .as_ref()
            .filter(|oauth| oauth.issuer == server.issuer)
            .and_then(|oauth| oauth.scope.as_deref()),
        server.scope.as_deref(),
    ])?;
    // Do not request offline_access alone; an empty request lets the issuer
    // apply its default scopes. Add it once actual resource scopes are known.
    scopes::union(&[
        scope.as_deref(),
        (server.offline_access && scope.is_some()).then_some("offline_access"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn granted_scopes_carry_over_only_from_the_same_issuer() {
        let mut connection = Connection::draft("Scoped", "https://mcp.test/mcp");
        connection.pending_scope = Some("deploy".into());
        connection.oauth = Some(crate::mcp::types::OAuthTokens {
            endpoint: connection.endpoint.clone(),
            access_token: "isolated-access".into(),
            refresh_token: None,
            expires_at: None,
            token_endpoint: "https://old.test/token".into(),
            client_id: "isolated-client".into(),
            client_auth_method: crate::mcp::types::ClientAuthMethod::None,
            client_secret: None,
            client_secret_expires_at: None,
            issuer: "https://old.test".into(),
            scope: Some("legacy:read".into()),
            resource: Some(connection.endpoint.clone()),
            github: false,
        });
        let mut server = discovery::Server {
            issuer: "https://new.test".into(),
            authorize: "https://new.test/authorize".into(),
            token: "https://new.test/token".into(),
            registration: None,
            resource: connection.endpoint.clone(),
            scope: Some("read".into()),
            offline_access: false,
            cimd: false,
            response_iss_required: false,
            auth_methods: None,
        };
        assert_eq!(
            requested_scope(&connection, &server).unwrap().as_deref(),
            Some("deploy read")
        );
        server.issuer = "https://old.test".into();
        assert_eq!(
            requested_scope(&connection, &server).unwrap().as_deref(),
            Some("deploy legacy:read read")
        );
    }

    #[test]
    fn offline_access_waits_for_actual_scopes_even_without_resource_scopes() {
        let mut connection = Connection::draft("Scoped", "https://mcp.test/mcp");
        let server = discovery::Server {
            issuer: "https://issuer.test".into(),
            authorize: "https://issuer.test/authorize".into(),
            token: "https://issuer.test/token".into(),
            registration: None,
            resource: connection.endpoint.clone(),
            scope: None,
            offline_access: true,
            cimd: false,
            response_iss_required: false,
            auth_methods: None,
        };
        assert_eq!(requested_scope(&connection, &server).unwrap(), None);
        connection.pending_scope = Some("deploy".into());
        assert_eq!(
            requested_scope(&connection, &server).unwrap().as_deref(),
            Some("deploy offline_access")
        );
    }

    fn send(port: u16, target: &str) -> String {
        use std::io::Write;
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
        let (code, landing) = wait(
            &listener,
            port,
            "right",
            "https://issuer.test",
            true,
            &cancel,
        )
        .unwrap();
        assert_eq!(code, "good");
        landing.authenticated();
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
