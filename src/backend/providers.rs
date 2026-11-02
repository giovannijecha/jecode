use super::{BackendInfo, anthropic, openai, openrouter};

static AVAILABLE: [BackendInfo; 5] = [
    BackendInfo {
        id: "openrouter",
        name: "OpenRouter",
        key_url: Some("https://openrouter.ai/settings/keys"),
        credential_env: openrouter::CREDENTIAL_ENV,
    },
    BackendInfo {
        id: "openai-api",
        name: "OpenAI API",
        key_url: Some("https://platform.openai.com/api-keys"),
        credential_env: openai::API_CREDENTIAL_ENV,
    },
    BackendInfo {
        id: "anthropic-api",
        name: "Anthropic API",
        key_url: Some("https://platform.claude.com/settings/keys"),
        credential_env: anthropic::API_CREDENTIAL_ENV,
    },
    BackendInfo {
        id: "openai",
        name: "OpenAI",
        key_url: None,
        credential_env: openai::CREDENTIAL_ENV,
    },
    BackendInfo {
        id: "anthropic",
        name: "Anthropic",
        key_url: None,
        credential_env: anthropic::CREDENTIAL_ENV,
    },
];

pub fn available() -> &'static [BackendInfo] {
    &AVAILABLE
}

/// The registered provider with this backend ID.
pub fn info(id: &str) -> Option<&'static BackendInfo> {
    AVAILABLE.iter().find(|provider| provider.id == id)
}

impl BackendInfo {
    /// API-key providers publish a key page; account providers sign in.
    pub fn uses_api_key(&self) -> bool {
        self.key_url.is_some()
    }

    /// The key's display name, without repeating "API" for API connections.
    pub fn key_label(&self) -> String {
        if self.name.ends_with(" API") {
            format!("{} key", self.name)
        } else {
            format!("{} API key", self.name)
        }
    }
}
