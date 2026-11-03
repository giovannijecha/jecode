use super::*;

fn headers(connection: Connection) -> Vec<(String, String)> {
    Client::new(connection)
        .headers("tools/list", None, None)
        .unwrap()
}

fn value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[test]
fn stock_github_adds_actions_to_default_tools_for_new_and_saved_connections() {
    for endpoint in [
        "https://api.githubcopilot.com/mcp",
        "https://api.githubcopilot.com/mcp/",
    ] {
        let mut connection = Connection::draft("GitHub", endpoint);
        connection.id = "mcp-saved".into();
        connection
            .headers
            .push(("X-MCP-Readonly".into(), "true".into()));
        let sent = headers(connection);
        assert_eq!(value(&sent, "X-MCP-Toolsets"), Some(GITHUB_TOOLSETS));
        assert_eq!(value(&sent, "X-MCP-Readonly"), Some("true"));
    }
}

#[test]
fn custom_tool_selection_and_other_endpoints_are_untouched() {
    for name in ["X-MCP-Toolsets", "x-mcp-tools", "X-Mcp-Exclude-Tools"] {
        let mut connection = Connection::draft("GitHub", "https://api.githubcopilot.com/mcp/");
        connection.headers.push((name.into(), "custom".into()));
        let sent = headers(connection);
        assert_eq!(sent.len(), 3);
        assert_eq!(value(&sent, name), Some("custom"));
    }
    for endpoint in [
        "https://api.githubcopilot.com/mcp/x/actions",
        "https://api.githubcopilot.com/mcp/?mode=custom",
        "https://mcp.example.com/mcp/",
    ] {
        let sent = headers(Connection::draft("GitHub", endpoint));
        assert_eq!(value(&sent, "X-MCP-Toolsets"), None);
    }
}
