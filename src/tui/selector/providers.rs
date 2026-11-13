use super::{Badge, Choice, OptionRow, ProviderGroup, Purpose, SETTINGS, Selector, row};

impl ProviderGroup {
    pub fn for_id(id: &str) -> Self {
        if crate::backend::info(id).is_some_and(|info| info.uses_api_key()) {
            Self::ApiKeys
        } else {
            Self::Credentials
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ApiKeys => "API keys",
            Self::Credentials => "Credentials",
        }
    }
}

impl Selector {
    pub fn provider_groups(
        services: &[crate::backend::Services],
        defaults: bool,
        current: &str,
        saved: &str,
        focus: Option<ProviderGroup>,
    ) -> Self {
        let options = [ProviderGroup::ApiKeys, ProviderGroup::Credentials]
            .into_iter()
            .filter_map(|group| {
                let members = services
                    .iter()
                    .filter(|service| ProviderGroup::for_id(service.id()) == group)
                    .collect::<Vec<_>>();
                (!members.is_empty()).then(|| OptionRow {
                    badge: if defaults && members.iter().any(|service| service.id() == saved) {
                        Some(Badge::Default)
                    } else if members.iter().any(|service| service.id() == current) {
                        Some(Badge::Current)
                    } else if members.iter().any(|service| service.id() == saved) {
                        Some(Badge::Default)
                    } else {
                        None
                    },
                    ..row(
                        group.label(),
                        members
                            .iter()
                            .filter_map(|service| crate::backend::info(service.id()))
                            .map(|info| info.name)
                            .collect::<Vec<_>>()
                            .join(" · "),
                        Choice::ProviderGroup(group),
                    )
                })
            })
            .collect();
        let mut menu = Self::new(
            if defaults {
                "Default provider"
            } else {
                "Provider"
            },
            Purpose::ProviderGroups { defaults },
            options,
        )
        .under_settings(defaults);
        menu.context = if defaults {
            "new conversations"
        } else {
            "this conversation"
        }
        .into();
        let focus =
            focus.unwrap_or_else(|| ProviderGroup::for_id(if defaults { saved } else { current }));
        menu.selected = menu
            .options
            .iter()
            .position(
                |option| matches!(option.choice, Choice::ProviderGroup(group) if group == focus),
            )
            .unwrap_or(0);
        menu
    }
    pub fn providers_settings() -> Self {
        let keys = crate::backend::available()
            .iter()
            .filter(|provider| provider.uses_api_key())
            .map(|provider| provider.name)
            .collect::<Vec<_>>()
            .join(" · ");
        let mut menu = Self::new(
            "Providers",
            Purpose::ProviderSettings,
            vec![
                row("API keys", keys, Choice::ApiKeys),
                row("Credentials", "OpenAI · Anthropic", Choice::Credentials),
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
    pub fn api_keys(configured: &[(&'static str, bool)]) -> Self {
        let mut menu = Self::new(
            "API keys",
            Purpose::ApiKeys { delete: None },
            configured
                .iter()
                .filter_map(|(id, saved)| {
                    crate::backend::info(id)
                        .filter(|provider| provider.uses_api_key())
                        .map(|provider| OptionRow {
                            badge: saved.then_some(Badge::Note("configured".into())),
                            ..row(provider.name, "API key", Choice::ApiKey(provider.id))
                        })
                })
                .collect(),
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
        options.push(row("Manage usage", "", Choice::ManageUsage));
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
        group: ProviderGroup,
        current: &str,
        saved: &str,
    ) -> Self {
        let options = services
            .iter()
            .map(|service| {
                let id = service.id();
                let registered = crate::backend::info(id);
                let name = registered.map_or(id, |entry| entry.name);
                let fact = if registered.is_some_and(|entry| entry.uses_api_key()) {
                    "API key"
                } else {
                    match id {
                        "openai" => "ChatGPT plan",
                        "anthropic" => "Claude plan",
                        _ => "",
                    }
                };
                OptionRow {
                    badge: if defaults && id == saved {
                        Some(Badge::Default)
                    } else if id == current {
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
            group.label(),
            Purpose::Providers { defaults, group },
            options,
        );
        menu.parent = Some(if defaults {
            "Default provider"
        } else {
            "Provider"
        });
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
