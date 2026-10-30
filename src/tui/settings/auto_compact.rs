use super::super::{App, Kind, held::Deferred};

impl App {
    /// Saves the threshold, then applies it to the active conversation. An
    /// invalid value keeps the panel open; a failed save keeps both unchanged.
    pub(in crate::tui) fn save_auto_compact(&mut self) {
        let text = &self.state.selector.as_ref().unwrap().editor.text;
        let text = text.trim();
        let percent = text
            .strip_suffix('%')
            .unwrap_or(text)
            .trim_end()
            .parse::<usize>()
            .map_err(|_| "Auto-compact must be a whole percentage from 1 to 99".to_string())
            .and_then(crate::context::auto_compact_percent);
        let percent = match percent {
            Ok(percent) => percent,
            Err(error) => return self.warn(&format!("{error} · previous setting kept")),
        };
        self.state.selector = None;
        let mut settings = self.config.settings.clone();
        settings.auto_compact = percent;
        let first_setup = !settings.has_model();
        let saved = if first_setup {
            Ok(())
        } else {
            self.config.store.save(&settings)
        };
        match saved {
            Ok(()) => {
                let previous = self.config.settings.auto_compact;
                self.config.settings = settings;
                self.defer(Deferred::AutoCompact);
                self.refresh_context();
                self.finish_setting_change(
                    Kind::Notice,
                    &format!(
                        "Auto-compact: {previous}% → {percent}% · {}",
                        if first_setup {
                            "this session · choose a default model to save"
                        } else {
                            "this and new conversations"
                        }
                    ),
                    vec![],
                );
            }
            Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
        }
    }
}
