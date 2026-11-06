use super::super::{App, Kind, theme};
use crate::config::Theme;

impl App {
    /// Saves the theme and applies it at once. The current choice closes without a notice.
    pub(in crate::tui) fn save_theme(&mut self, chosen: Theme) {
        let previous = self.config.settings.theme;
        if chosen == previous {
            self.return_to_settings();
            return;
        }
        let mut settings = self.config.settings.clone();
        settings.theme = chosen;
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
        self.apply_theme();
        self.finish_setting_change(
            Kind::Notice,
            &format!(
                "Theme: {} → {} · {}",
                previous.name(),
                chosen.name(),
                if first_setup {
                    "this session · choose a default model to save"
                } else {
                    "this and new conversations"
                }
            ),
            vec![],
        );
    }

    /// The terminal reported its background, which only Auto follows.
    pub(in crate::tui) fn terminal_background(&mut self, light: bool) {
        self.light_terminal = light;
        self.apply_theme();
    }

    pub(in crate::tui) fn apply_theme(&mut self) {
        let light = match self.config.settings.theme {
            Theme::Auto => self.light_terminal,
            Theme::Dark => false,
            Theme::Light => true,
        };
        if theme::set_light(light) {
            self.renderer.repaint();
        }
    }
}
