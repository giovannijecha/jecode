//! Neutral MCP connection, discovery and failure data shared by UI and tools.

use crate::json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    None,
    Token,
    Browser,
    GitHubDevice,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OAuthTokens {
    pub endpoint: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
    pub token_endpoint: String,
    pub client_id: String,
    pub issuer: String,
    pub scope: Option<String>,
    pub resource: Option<String>,
    pub github: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub enabled: bool,
    pub method: AuthMethod,
    pub token: String,
    pub oauth: Option<OAuthTokens>,
    pub oauth_client_id: Option<String>,
    pub timeout_secs: u64,
    pub headers: Vec<(String, String)>,
}

impl Connection {
    pub fn draft(name: &str, endpoint: &str) -> Self {
        Self {
            id: String::new(),
            name: name.into(),
            endpoint: endpoint.into(),
            enabled: true,
            method: AuthMethod::None,
            token: String::new(),
            oauth: None,
            oauth_client_id: None,
            timeout_secs: 60,
            headers: Vec::new(),
        }
    }

    pub fn bearer(&self) -> Option<&str> {
        match self.method {
            AuthMethod::None => None,
            AuthMethod::Token => (!self.token.is_empty()).then_some(self.token.as_str()),
            AuthMethod::Browser | AuthMethod::GitHubDevice => {
                self.oauth.as_ref().map(|o| o.access_token.as_str())
            }
        }
    }

    pub fn protect(&self, redactor: &crate::redact::Redactor) {
        redactor.add_secret(self.token.clone());
        for (_, value) in &self.headers {
            redactor.add_secret(value.clone());
        }
        if let Some(tokens) = &self.oauth {
            redactor.add_secret(tokens.access_token.clone());
            if let Some(refresh) = &tokens.refresh_token {
                redactor.add_secret(refresh.clone());
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Tool {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub input_schema: Value,
    pub annotations: Option<Value>,
}

#[derive(Clone, Debug, Default)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub instructions: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub server: ServerInfo,
    pub tools: Vec<Tool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Configuration,
    Authentication,
    Transport,
    Protocol,
    Remote,
    Cancelled,
    UnknownOutcome,
    Unsupported,
}

#[derive(Clone)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub status: Option<u16>,
    /// Challenges are inspected by authentication, never included in tool output.
    pub headers: Vec<(String, String)>,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status: None,
            headers: Vec::new(),
        }
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpError")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .field("status", &self.status)
            .finish()
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}
