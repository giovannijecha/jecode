use super::super::{App, Kind};
use crate::backend::CacheRetention;

impl App {
    /// Saves the lifetime, then applies it from the next request of this and
    /// every other conversation. The current choice closes without a notice.
    pub(in crate::tui) fn save_prompt_cache(&mut self, retention: CacheRetention) {
        let previous = self.config.settings.cache_retention;
        if retention == previous {
            self.return_to_settings();
            return;
        }
        let mut settings = self.config.settings.clone();
        settings.cache_retention = retention;
        let first_setup = !settings.has_model();
        let saved = if first_setup {
            Ok(())
        } else {
            self.config.store.save(&settings)
        };
        if let Err(error) = saved {
            self.finish_setting(Kind::Error, &error, vec![]);
            return;
        }
        self.config.settings = settings;
        self.facts().cache_retention.store(
            retention == CacheRetention::Long,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.finish_setting_change(
            Kind::Notice,
            &format!(
                "Prompt cache: {} → {} · {}",
                previous.name(),
                retention.name(),
                if first_setup {
                    "this session · choose a default model to save"
                } else {
                    "this and new conversations"
                }
            ),
            vec![],
        );
    }
}
