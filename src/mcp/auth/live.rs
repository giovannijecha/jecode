//! Explicit browser approval against a selected real MCP provider.

use super::{Update, refresh, sign_in};
use crate::cancel::Cancellation;
use crate::mcp::types::{AuthMethod, Connection};
use crate::mcp::{Client, Store};
use crate::test_support::Directory;

#[test]
#[ignore = "requires an explicitly selected provider and interactive browser approval"]
fn provider_oauth_roundtrip() {
    let provider = std::env::var("JECODE_MCP_LIVE_PROVIDER")
        .expect("set JECODE_MCP_LIVE_PROVIDER to github, supabase, cloudflare or vercel");
    let (endpoint, method) = match provider.as_str() {
        "github" => (
            "https://api.githubcopilot.com/mcp/",
            AuthMethod::GitHubDevice,
        ),
        "supabase" => ("https://mcp.supabase.com/mcp", AuthMethod::Browser),
        "cloudflare" => ("https://mcp.cloudflare.com/mcp", AuthMethod::Browser),
        "vercel" => ("https://mcp.vercel.com/", AuthMethod::Browser),
        _ => panic!("unsupported live provider"),
    };
    let fixture = Directory::new();
    let store = Store::new(fixture.path().to_path_buf());
    let cancel = Cancellation::default();
    let mut connection = Connection::draft("Explicit OAuth verification", endpoint);
    connection.method = method;
    println!("Starting {provider} OAuth with an isolated credential store.");
    let signed_in = sign_in(connection, &cancel, &mut |update| {
        match update {
            Update::Browser(url) => {
                println!("Opening {provider} sign-in in the browser.");
                crate::auth::open_browser(&url)?;
            }
            Update::DeviceCode { url, code } => {
                println!("Open {url} and enter the one-time code {code}.");
                crate::auth::open_browser(&url)?;
            }
            Update::Waiting => println!("Waiting for browser approval."),
        }
        Ok(())
    })
    .unwrap_or_else(|error| panic!("OAuth sign-in failed: {}", error.message));
    let saved = store
        .save(&signed_in)
        .unwrap_or_else(|error| panic!("OAuth storage failed: {}", error.message));
    let catalog = Client::new(saved.clone())
        .catalog(&cancel)
        .unwrap_or_else(|error| panic!("Authenticated MCP discovery failed: {}", error.message));
    assert!(!catalog.tools.is_empty(), "provider returned no tools");
    println!(
        "{provider}: browser sign-in and {} discovered tools passed.",
        catalog.tools.len()
    );
    if provider == "github" {
        assert!(
            catalog
                .tools
                .iter()
                .any(|tool| tool.name.starts_with("actions_") || tool.name.contains("workflow")),
            "GitHub Actions toolset is unavailable; check installation permission approval"
        );
    }
    if saved
        .oauth
        .as_ref()
        .is_some_and(|tokens| tokens.refresh_token.is_some())
    {
        let refreshed = refresh(&saved, &cancel)
            .unwrap_or_else(|error| panic!("OAuth refresh failed: {}", error.message));
        let refreshed = store
            .save(&refreshed)
            .unwrap_or_else(|error| panic!("Refreshed OAuth storage failed: {}", error.message));
        let catalog = Client::new(refreshed)
            .catalog(&cancel)
            .unwrap_or_else(|error| panic!("Discovery after refresh failed: {}", error.message));
        assert!(!catalog.tools.is_empty());
        println!("{provider}: refresh and subsequent MCP discovery passed.");
    } else {
        println!("{provider}: server did not issue a refresh token.");
    }
    println!("Live verification complete; the isolated local credential store will be removed.");
}
