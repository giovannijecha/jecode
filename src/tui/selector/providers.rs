use super::{Badge, Choice, OptionRow, Purpose, SETTINGS, Selector, row};

impl Selector {
    pub fn providers_settings() -> Self {
        let mut menu = Self::new(
            "Providers",
            Purpose::ProviderSettings,
            vec![
                row("Credentials", "OpenAI · Anthropic", Choice::Credentials),
                row("API keys", "OpenRouter", Choice::ApiKeys),
            ],
        );
        menu.parent = Some(SETTINGS);
        menu
    }
    pub fn credentials(openai_configured: bool, anthropic_configured: bool) -> Self {
        let mut menu = Self::new(
            "Credentials",
            Purpose::Credentials,
            vec![
                OptionRow {
                    badge: openai_configured.then_some(Badge::Note("configured".into())),
                    ..row("OpenAI", "ChatGPT plan", Choice::Provider("openai"))
                },
                OptionRow {
                    badge: anthropic_configured.then_some(Badge::Note("configured".into())),
                    ..row("Anthropic", "Claude plan", Choice::Provider("anthropic"))
                },
            ],
        );
        menu.parent = Some("Providers");
        menu
    }
    pub fn api_keys(configured: bool) -> Self {
        let mut menu = Self::new(
            "API keys",
            Purpose::ApiKeys,
            vec![OptionRow {
                badge: configured.then_some(Badge::Note("configured".into())),
                ..row("OpenRouter", "API key", Choice::OpenRouterKey)
            }],
        );
        menu.parent = Some("Providers");
        menu
    }
    pub fn accounts(
        provider: &'static str,
        accounts: &[crate::auth::Account],
        active: Option<&str>,
    ) -> Self {
        let mut options: Vec<_> = accounts
            .iter()
            .enumerate()
            .map(|(index, account)| OptionRow {
                badge: if active == Some(account.id.as_str()) {
                    Some(Badge::Current)
                } else if !account.enabled {
                    Some(Badge::Note("unavailable".into()))
                } else {
                    None
                },
                search: account.detail.clone(),
                ..row(
                    account.label.clone(),
                    account.detail.clone(),
                    Choice::Account(index),
                )
            })
            .collect();
        options.push(row("Add account", "", Choice::AddAccount));
        let mut menu = Self::new(provider_name(provider), Purpose::Accounts, options);
        menu.parent = Some("Credentials");
        menu.context = if provider == "anthropic" {
            "Claude accounts"
        } else {
            "ChatGPT accounts"
        }
        .into();
        menu
    }
    pub fn account(provider: &'static str, account: &crate::auth::Account, active: bool) -> Self {
        let mut options = vec![OptionRow {
            badge: active.then_some(Badge::Current),
            ..row("Use account", &account.detail, Choice::Account(0))
        }];
        if provider == "openai" {
            options.push(row("Manage usage", "", Choice::ManageUsage));
        }
        options.push(row("Sign out", "", Choice::SignOut));
        let mut menu = Self::new(
            "Account",
            Purpose::Account {
                confirm_sign_out: false,
            },
            options,
        );
        menu.parent = Some(provider_name(provider));
        menu.context = account.label.clone();
        menu
    }
    pub fn providers(
        services: &[crate::backend::Services],
        defaults: bool,
        current: &str,
        saved: &str,
    ) -> Self {
        let options = services
            .iter()
            .map(|service| {
                let id = service.id();
                let (name, fact) = match id {
                    "openrouter" => ("OpenRouter", "API key"),
                    "openai" => ("OpenAI", "ChatGPT plan"),
                    "anthropic" => ("Anthropic", "Claude plan"),
                    _ => (id, ""),
                };
                OptionRow {
                    badge: if id == current {
                        Some(Badge::Current)
                    } else if id == saved {
                        Some(Badge::Default)
                    } else {
                        None
                    },
                    ..row(name, fact, Choice::Provider(id))
                }
            })
            .collect();
        let mut menu = Self::new(
            if defaults {
                "Default provider"
            } else {
                "Provider"
            },
            Purpose::Providers { defaults },
            options,
        )
        .under_settings(defaults);
        menu.context = if defaults {
            "new conversations"
        } else {
            "this conversation"
        }
        .into();
        menu
    }
}

fn provider_name(provider: &str) -> &'static str {
    if provider == "anthropic" {
        "Anthropic"
    } else {
        "OpenAI"
    }
}
