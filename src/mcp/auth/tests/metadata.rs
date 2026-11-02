use super::*;

fn authorization(origin: &str, issuer: &str) -> String {
    format!(
        r#"{{"issuer":"{issuer}","authorization_endpoint":"{origin}/authorize","token_endpoint":"{origin}/token","registration_endpoint":"{origin}/register","code_challenge_methods_supported":["S256"]}}"#
    )
}

#[test]
fn authorization_metadata_skips_incomplete_json_for_oidc_candidate() {
    let server = serve(4, |origin, path| match path {
        "/mcp" => ("401 Unauthorized", "{}".into(), String::new()),
        "/.well-known/oauth-protected-resource/mcp" => (
            "200 OK",
            format!(r#"{{"resource":"{origin}/mcp","authorization_servers":["{origin}"]}}"#),
            String::new(),
        ),
        "/.well-known/oauth-authorization-server" => ("200 OK", "{}".into(), String::new()),
        "/.well-known/openid-configuration" => {
            ("200 OK", authorization(origin, origin), String::new())
        }
        _ => panic!("unexpected fixture path {path}"),
    });
    assert_eq!(
        checked(&server),
        vec![AuthMethod::Token, AuthMethod::Browser]
    );
    server.finish();
}

#[test]
fn authorization_metadata_skips_wrong_issuer_and_missing_pkce() {
    let server = serve(5, |origin, path| match path {
        "/mcp" => ("401 Unauthorized", "{}".into(), String::new()),
        "/.well-known/oauth-protected-resource/mcp" => (
            "200 OK",
            format!(r#"{{"resource":"{origin}/mcp","authorization_servers":["{origin}/issuer"]}}"#),
            String::new(),
        ),
        "/.well-known/oauth-authorization-server/issuer" => {
            ("200 OK", authorization(origin, origin), String::new())
        }
        "/.well-known/openid-configuration/issuer" => (
            "200 OK",
            format!(
                r#"{{"issuer":"{origin}/issuer","authorization_endpoint":"{origin}/authorize","token_endpoint":"{origin}/token"}}"#
            ),
            String::new(),
        ),
        "/issuer/.well-known/openid-configuration" => (
            "200 OK",
            authorization(origin, &format!("{origin}/issuer")),
            String::new(),
        ),
        _ => panic!("unexpected fixture path {path}"),
    });
    assert_eq!(
        checked(&server),
        vec![AuthMethod::Token, AuthMethod::Browser]
    );
    server.finish();
}

#[test]
fn authorization_metadata_rejects_candidates_without_pkce() {
    let server = serve(3, |origin, path| match path {
        "/.well-known/oauth-protected-resource/mcp" => (
            "200 OK",
            format!(r#"{{"resource":"{origin}/mcp","authorization_servers":["{origin}"]}}"#),
            String::new(),
        ),
        "/.well-known/oauth-authorization-server" => (
            "200 OK",
            format!(
                r#"{{"issuer":"{origin}","authorization_endpoint":"{origin}/authorize","token_endpoint":"{origin}/token"}}"#
            ),
            String::new(),
        ),
        "/.well-known/openid-configuration" => ("404 Not Found", "{}".into(), String::new()),
        _ => panic!("unexpected fixture path {path}"),
    });
    let connection = Connection::draft("fixture", &format!("{}/mcp", server.endpoint));
    let error = discovery::discover_challenged(&connection, &Cancellation::default(), &[])
        .err()
        .expect("an issuer without PKCE must be rejected");
    assert!(error.message.contains("PKCE S256"));
    server.finish();
}

#[test]
fn protected_resource_metadata_skips_wrong_resource_identity() {
    let server = serve(4, |origin, path| match path {
        "/mcp" => ("401 Unauthorized", "{}".into(), String::new()),
        "/.well-known/oauth-protected-resource/mcp" => (
            "200 OK",
            format!(r#"{{"resource":"{origin}/other","authorization_servers":["{origin}"]}}"#),
            String::new(),
        ),
        "/.well-known/oauth-protected-resource" => (
            "200 OK",
            format!(r#"{{"resource":"{origin}/mcp","authorization_servers":["{origin}"]}}"#),
            String::new(),
        ),
        "/.well-known/oauth-authorization-server" => {
            ("200 OK", authorization(origin, origin), String::new())
        }
        _ => panic!("unexpected fixture path {path}"),
    });
    assert_eq!(
        checked(&server),
        vec![AuthMethod::Token, AuthMethod::Browser]
    );
    server.finish();
}
