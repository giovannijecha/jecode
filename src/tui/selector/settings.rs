use super::{Badge, CONTEXT, Choice, OptionRow, Purpose, SETTINGS, Selector, Setting, row};
use crate::{
    backend::{CacheRetention, Routing},
    effort::Effort,
};
use std::path::Path;

fn setting(name: &str, value: String, setting: Setting) -> OptionRow {
    row(name, value, Choice::Setting(setting))
}

impl Selector {
    /// `routing` is the default model's saved routing.
    pub fn settings(
        model: &str,
        effort: Effort,
        auto_compact: u8,
        cache_retention: CacheRetention,
        routing: &Routing,
        permissions: crate::permissions::Level,
        _config_path: &Path,
    ) -> Self {
        let mut menu = Self::new(
            SETTINGS,
            Purpose::Settings,
            vec![
                setting(
                    "Default model",
                    if model.is_empty() {
                        "Not selected".into()
                    } else {
                        model.into()
                    },
                    Setting::Model,
                ),
                setting("Default effort", effort.name().into(), Setting::Effort),
                setting("Default routing", routing.label(), Setting::Routing),
                setting(
                    "Default permissions",
                    permissions.name().into(),
                    Setting::Permissions,
                ),
                setting(
                    CONTEXT,
                    format!("{auto_compact}% · {}", cache_retention.name()),
                    Setting::Context,
                ),
                setting("Providers", String::new(), Setting::Providers),
                setting("MCP", String::new(), Setting::Mcp),
                setting("LSP", String::new(), Setting::Lsp),
            ],
        );
        "saved defaults".clone_into(&mut menu.context);
        menu
    }

    /// Settings › Context: the preferences that shape what each request
    /// carries and how long the provider keeps it cached.
    pub fn context_settings(auto_compact: u8, cache_retention: CacheRetention) -> Self {
        Self::new(
            CONTEXT,
            Purpose::Context,
            vec![
                setting(
                    "Auto-compact at",
                    format!("{auto_compact}%"),
                    Setting::AutoCompact,
                ),
                setting(
                    "Prompt cache",
                    cache_retention.name().into(),
                    Setting::PromptCache,
                ),
            ],
        )
        .under_settings(true)
    }

    /// One row per lifetime, the ` current ` chip and the focus on `current`.
    pub fn prompt_cache(current: CacheRetention) -> Self {
        let options = CacheRetention::ALL
            .into_iter()
            .map(|retention| OptionRow {
                badge: (retention == current).then_some(Badge::Current),
                ..row(
                    retention.name(),
                    retention.fact(),
                    Choice::Retention(retention),
                )
            })
            .collect();
        let mut menu = Self::new("Prompt cache", Purpose::PromptCache, options).under_context();
        menu.selected = CacheRetention::ALL
            .iter()
            .position(|retention| *retention == current)
            .unwrap_or(0);
        menu
    }

    /// The settings row of `setting`, to restore focus on return.
    pub fn setting_index(&self, setting: Setting) -> usize {
        self.options
            .iter()
            .position(
                |option| matches!(option.choice, Choice::Setting(choice) if choice == setting),
            )
            .unwrap_or(0)
    }
}
