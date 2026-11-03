use super::*;

#[test]
fn browser_pkce_and_refresh_stay_on_discovered_issuer_and_resource() {
    browser_auth_round_trip(Some("none"), "none", None);
    browser_auth_round_trip(Some("client_secret_basic"), "client_secret_basic", None);
    browser_auth_round_trip(Some("client_secret_post"), "client_secret_post", None);
    browser_auth_round_trip(Some("none"), "client_secret_basic", None);
    browser_auth_round_trip(None, "client_secret_basic", None);
    browser_auth_round_trip(Some("none"), "none", Some("repo:read"));
}

fn browser_auth_round_trip(
    advertised: Option<&'static str>,
    method: &'static str,
    token_scope: Option<&'static str>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let issuer = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let endpoint = format!("{issuer}/mcp?project_ref=fixture");
    let server_issuer = issuer.clone();
    let server_resource = format!("{issuer}/mcp");
    let expected_resource = server_resource.clone();
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut auth_count = 0;
        let mut refresh_count = 0;
        for _ in 0..9 {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("OAuth fixture accept: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if let Some(pos) = request.windows(4).position(|p| p == b"\r\n\r\n") {
                    let head = std::str::from_utf8(&request[..pos]).unwrap();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= pos + 4 + length {
                        break;
                    }
                }
            }
            let text = std::str::from_utf8(&request).unwrap();
            let path = text.split_whitespace().nth(1).unwrap();
            let route = path.split('?').next().unwrap_or(path);
            if route == "/mcp" {
                assert!(text.contains("MCP-Protocol-Version: 2026-07-28"));
                assert!(text.contains("Mcp-Method: server/discover"));
            }
            let body = text
                .split_once("\r\n\r\n")
                .map(|(_, body)| body)
                .unwrap_or("");
            let (status, reply, extra) = match route {
                "/mcp" => (
                    "401 Unauthorized",
                    "{}".into(),
                    format!(
                        "WWW-Authenticate: Bearer resource_metadata=\"{server_issuer}/resource-info\"\r\n"
                    ),
                ),
                "/resource-info" => (
                    "200 OK",
                    format!(
                        r#"{{"resource":"{server_resource}","authorization_servers":["{server_issuer}"],"scopes_supported":["repo:read"]}}"#
                    ),
                    String::new(),
                ),
                "/.well-known/oauth-authorization-server" => (
                    "200 OK",
                    format!(
                        r#"{{"issuer":"{server_issuer}","authorization_endpoint":"{server_issuer}/authorize?audience=fixture","token_endpoint":"{server_issuer}/token","registration_endpoint":"{server_issuer}/register","authorization_response_iss_parameter_supported":true,"code_challenge_methods_supported":["S256"]{}}}"#,
                        advertised.map_or(String::new(), |method| format!(
                            r#", "token_endpoint_auth_methods_supported":["{method}"]"#
                        ))
                    ),
                    String::new(),
                ),
                "/register" => {
                    assert!(body.contains(&format!(
                        "\"token_endpoint_auth_method\":\"{}\"",
                        advertised.unwrap_or("client_secret_basic")
                    )));
                    (
                        "201 Created",
                        if method == "none" {
                            r#"{"client_id":"synthetic-client"}"#.into()
                        } else {
                            format!(
                                r#"{{"client_id":"client:one","client_secret":"s:e+c","client_secret_expires_at":0{}}}"#,
                                advertised.map_or(String::new(), |_| format!(
                                    r#", "token_endpoint_auth_method":"{method}""#
                                ))
                            )
                        },
                        String::new(),
                    )
                }
                "/token" if body.contains("grant_type=authorization_code") => {
                    auth_count += 1;
                    assert!(body.contains("code_verifier="));
                    assert!(body.contains("resource="));
                    assert_client_auth(text, body, method);
                    (
                        "200 OK",
                        format!(
                            r#"{{"access_token":"access-one","refresh_token":"refresh-one","expires_in":120,"token_type":"Bearer"{}}}"#,
                            token_scope
                                .map_or(String::new(), |scope| format!(r#", "scope":"{scope}""#))
                        ),
                        String::new(),
                    )
                }
                "/token" if body.contains("grant_type=refresh_token") => {
                    refresh_count += 1;
                    assert!(body.contains("refresh_token=refresh-one"));
                    assert!(body.contains("resource="));
                    assert_client_auth(text, body, method);
                    ("200 OK",r#"{"access_token":"access-two","refresh_token":"refresh-two","expires_in":120,"token_type":"Bearer"}"#.into(),String::new())
                }
                _ => panic!("unexpected OAuth fixture request: {path}"),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{reply}",
                reply.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        assert_eq!(auth_count, 1);
        assert_eq!(refresh_count, 1);
    });
    let mut connection = Connection::draft("fixture", &endpoint);
    connection.method = AuthMethod::Browser;
    connection.pending_scope = Some("new:write".into());
    connection.oauth = Some(OAuthTokens {
        endpoint: endpoint.clone(),
        access_token: "prior-access".into(),
        refresh_token: None,
        expires_at: None,
        token_endpoint: format!("{issuer}/token"),
        client_id: "prior-client".into(),
        client_auth_method: ClientAuthMethod::None,
        client_secret: None,
        client_secret_expires_at: None,
        issuer: issuer.clone(),
        scope: Some("granted:read".into()),
        resource: Some(format!("{issuer}/mcp")),
        github: false,
    });
    let expected_issuer = issuer.clone();
    let signed=sign_in(connection,&Cancellation::default(),&mut |update|{
        if let Update::Browser(url)=update {
            assert!(url.contains("audience=fixture&response_type=code"));
            let params=wire::query(url.split_once('?').unwrap().1).unwrap();
            assert_eq!(params.get("scope").map(String::as_str), Some("new:write granted:read repo:read"));
            let redirect=params.get("redirect_uri").unwrap().clone();
            let state=params.get("state").unwrap().clone();
            let target=redirect.strip_prefix("http://127.0.0.1:").unwrap();
            let port=target.split('/').next().unwrap().parse::<u16>().unwrap();
            let issuer=wire::encode(&expected_issuer);
            std::thread::spawn(move||{
                std::thread::sleep(Duration::from_millis(100));
                let mut stream=std::net::TcpStream::connect(("127.0.0.1",port)).unwrap();
                let request=format!("GET /callback?code=synthetic-code&state={state}&iss={issuer} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
                stream.write_all(request.as_bytes()).unwrap();
            });
        }
        Ok(())
    }).unwrap();
    assert_eq!(signed.oauth.as_ref().unwrap().access_token, "access-one");
    // The registered client belongs to this sign-in's redirect, not the user override.
    assert_eq!(
        signed.oauth.as_ref().unwrap().client_id,
        if method == "none" {
            "synthetic-client"
        } else {
            "client:one"
        }
    );
    assert_eq!(
        signed.oauth.as_ref().unwrap().client_secret.as_deref(),
        if method == "none" {
            None
        } else {
            Some("s:e+c")
        }
    );
    assert_eq!(signed.oauth_client_id, None);
    assert_eq!(signed.pending_scope, None);
    assert_eq!(
        signed.oauth.as_ref().unwrap().scope.as_deref(),
        token_scope.or(Some("new:write granted:read repo:read"))
    );
    assert_eq!(
        signed.oauth.as_ref().unwrap().resource.as_deref(),
        Some(expected_resource.as_str())
    );
    let mut changed = signed.clone();
    changed.endpoint = format!("{issuer}/mcp?project_ref=other");
    assert!(crate::mcp::config::validate(&changed).is_err());
    assert!(refresh(&changed, &Cancellation::default()).is_err());
    let renewed = refresh(&signed, &Cancellation::default()).unwrap();
    assert_eq!(renewed.oauth.as_ref().unwrap().access_token, "access-two");
    assert_eq!(
        renewed.oauth.as_ref().unwrap().refresh_token.as_deref(),
        Some("refresh-two")
    );
    assert_eq!(
        renewed.oauth.as_ref().unwrap().client_secret.as_deref(),
        signed.oauth.as_ref().unwrap().client_secret.as_deref()
    );
    assert_eq!(
        renewed.oauth.as_ref().unwrap().scope,
        signed.oauth.as_ref().unwrap().scope
    );
    worker.join().unwrap();
}

fn assert_client_auth(request: &str, body: &str, method: &str) {
    match method {
        "none" => {
            assert!(body.contains("client_id=synthetic-client"));
            assert!(!request.contains("Authorization: Basic"));
        }
        "client_secret_basic" => {
            let encoded = crate::attachments::base64::encode(b"client%3Aone:s%3Ae%2Bc");
            assert!(request.contains(&format!("Authorization: Basic {encoded}")));
            assert!(!body.contains("client_id="));
            assert!(!body.contains("client_secret="));
        }
        "client_secret_post" => {
            assert!(body.contains("client_id=client%3Aone"));
            assert!(body.contains("client_secret=s%3Ae%2Bc"));
            assert!(!request.contains("Authorization: Basic"));
        }
        _ => unreachable!(),
    }
}
