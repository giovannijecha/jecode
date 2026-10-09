use super::App;
use std::io::{self, Write};

impl App {
    pub(super) fn print_exit(&self) {
        let messages = self.archive.messages.lock().unwrap();
        let prompts = messages
            .iter()
            .filter(|message| {
                message.get("role").and_then(crate::json::Value::as_str) == Some("user")
            })
            .count();
        let tools = messages
            .iter()
            .filter(|message| {
                message.get("role").and_then(crate::json::Value::as_str) == Some("tool")
            })
            .count();
        let id = self
            .persistence
            .as_ref()
            .filter(|_| prompts > 0)
            .map(crate::sessions::Handle::id);
        let text = summary(prompts, tools, id.as_deref(), self.save_error.is_none());
        let _ = writeln!(io::stdout(), "{text}");
    }
}

fn summary(prompts: usize, tools: usize, id: Option<&str>, saved: bool) -> String {
    let mut text = format!("Jecode closed: {prompts} prompts, {tools} tools");
    if !saved {
        text.push_str("; autosave failed");
    } else if let Some(id) = id.filter(|_| prompts > 0) {
        text.push_str(&format!("\nResume: jecode resume {id}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_printed_resume_command_is_accepted_by_the_cli() {
        let text = summary(1, 0, Some("fixture-id"), true);
        let command = text
            .lines()
            .find_map(|line| line.strip_prefix("Resume: "))
            .unwrap();
        let arguments = command
            .split_whitespace()
            .skip(1)
            .map(std::ffi::OsString::from);
        assert!(matches!(
            crate::cli::parse(arguments),
            Ok(crate::cli::Action::Resume { id: Some(id), plain: false }) if id == "fixture-id"
        ));
    }

    #[test]
    fn exit_is_a_short_summary_with_a_resume_command_only_after_a_successful_save() {
        assert_eq!(
            summary(2, 3, Some("fixture-id"), true),
            "Jecode closed: 2 prompts, 3 tools\nResume: jecode resume fixture-id"
        );
        assert_eq!(
            summary(2, 3, Some("fixture-id"), false),
            "Jecode closed: 2 prompts, 3 tools; autosave failed"
        );
        assert_eq!(summary(0, 0, None, true).lines().count(), 1);
        assert_eq!(summary(0, 0, Some("unsaved-id"), true).lines().count(), 1);
    }
}
