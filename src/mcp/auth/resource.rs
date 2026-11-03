/// Match an advertised OAuth resource to its configured MCP endpoint.
/// Only the optional slash at an HTTP origin is interchangeable.
pub(in crate::mcp) fn matches_endpoint(resource: &str, endpoint: &str) -> bool {
    resource == endpoint
        || root(resource)
            .zip(root(endpoint))
            .is_some_and(|(a, b)| a == b)
}

fn root(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty()
        || authority
            .bytes()
            .any(|byte| matches!(byte, b'/' | b'?' | b'#'))
    {
        return None;
    }
    Some(url.strip_suffix('/').unwrap_or(url))
}

#[cfg(test)]
mod tests {
    use super::matches_endpoint;

    #[test]
    fn root_slash_is_the_only_alias() {
        assert!(matches_endpoint(
            "https://mcp.vercel.com/",
            "https://mcp.vercel.com"
        ));
        assert!(matches_endpoint(
            "http://127.0.0.1:8080",
            "http://127.0.0.1:8080/"
        ));
        assert!(matches_endpoint(
            "https://mcp.cloudflare.com/mcp",
            "https://mcp.cloudflare.com/mcp"
        ));
        for (resource, endpoint) in [
            ("https://mcp.vercel.com/", "https://mcp.vercel.com/mcp"),
            ("https://mcp.vercel.com/mcp/", "https://mcp.vercel.com/mcp"),
            ("https://mcp.vercel.com/", "https://mcp.vercel.com/?team=a"),
            ("https://mcp.vercel.com", "https://mcp.vercel.com?team=a"),
            ("https://mcp.vercel.com/", "https://other.vercel.com"),
        ] {
            assert!(
                !matches_endpoint(resource, endpoint),
                "{resource} {endpoint}"
            );
        }
    }

    #[test]
    fn stored_sessions_keep_the_discovery_resource_rules() {
        use crate::json::Value;
        use crate::mcp::config;

        for (endpoint, resource, accepted) in [
            ("https://server.test", "https://server.test/", true),
            ("https://server.test/", "https://server.test", true),
            (
                "https://server.test/mcp?project_ref=one",
                "https://server.test/mcp",
                true,
            ),
            (
                "https://server.test/mcp?project_ref=one",
                "https://server.test/mcp?project_ref=two",
                false,
            ),
            ("https://server.test/mcp", "https://server.test/mcp/", false),
            ("https://server.test/", "https://other.test/", false),
        ] {
            let value = Value::object([
                ("id", Value::string("mcp-fixture")),
                ("name", Value::string("Resource fixture")),
                ("endpoint", Value::string(endpoint)),
                ("method", Value::string("browser")),
                (
                    "oauth",
                    Value::object([
                        ("access_token", Value::string("isolated-access")),
                        ("endpoint", Value::string(endpoint)),
                        ("client_id", Value::string("isolated-client")),
                        ("issuer", Value::string("https://issuer.test")),
                        ("token_endpoint", Value::string("https://issuer.test/token")),
                        ("resource", Value::string(resource)),
                        ("github", Value::Bool(false)),
                    ]),
                ),
            ]);
            let connection = config::read(&value);
            assert_eq!(connection.is_ok(), accepted, "{endpoint} {resource}");
            if let Ok(connection) = connection {
                assert!(config::read(&config::write(&connection, Some(value))).is_ok());
            }
        }
    }
}
