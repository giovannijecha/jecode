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
