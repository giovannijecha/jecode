use super::{Choice, Purpose, SETTINGS, Selector, Setting, row};
use crate::{backend::Routing, effort::Effort};
use std::path::Path;

impl Selector {
    /// `routing` is the default model's saved routing.
    pub fn settings(
        model: &str,
        effort: Effort,
        auto_compact: u8,
        routing: &Routing,
        permissions: crate::permissions::Level,
        _config_path: &Path,
    ) -> Self {
        let setting =
            |name: &str, value: String, setting| row(name, value, Choice::Setting(setting));
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
                    "Auto-compact at",
                    format!("{auto_compact}%"),
                    Setting::AutoCompact,
                ),
                setting("Providers", String::new(), Setting::Providers),
            ],
        );
        "saved defaults".clone_into(&mut menu.context);
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
