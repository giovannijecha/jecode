use super::App;
use std::io::{self, Write};
use std::time::Duration;

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
        let text = summary(
            prompts,
            tools,
            self.started.elapsed(),
            id.as_deref(),
            self.save_error.is_none(),
        );
        let _ = writeln!(io::stdout(), "{text}");
    }
}

/// The normal screen keeps terminal colours: `>_` in ANSI magenta, `jecode`
/// bold, the facts dim, and the resume command in the default foreground,
/// aligned under the name. Only SGR 1, 2 and 35 are used.
fn summary(
    prompts: usize,
    tools: usize,
    elapsed: Duration,
    id: Option<&str>,
    saved: bool,
) -> String {
    let mut facts = vec![
        "closed".to_owned(),
        super::text::counted(prompts, "prompt"),
        super::text::counted(tools, "tool"),
        super::activity::duration(elapsed),
    ];
    if !saved {
        facts.push("autosave failed".into());
    }
    let mut text = format!(
        "\x1b[35m>_\x1b[0m \x1b[1mjecode\x1b[0m \x1b[2m{}\x1b[0m",
        facts.join(" · ")
    );
    if let Some(id) = id.filter(|_| saved && prompts > 0) {
        text.push_str(&format!("\n   jecode resume {id}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> String {
        let mut plain = String::new();
        let mut rest = text;
        while let Some(start) = rest.find('\x1b') {
            plain.push_str(&rest[..start]);
            let end = rest[start..].find('m').unwrap();
            rest = &rest[start + end + 1..];
        }
        plain + rest
    }

    #[test]
    fn the_printed_resume_command_is_accepted_by_the_cli() {
        let text = summary(1, 0, Duration::ZERO, Some("fixture-id"), true);
        let command = text.lines().nth(1).unwrap().trim();
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
        let elapsed = Duration::from_secs(252);
        assert_eq!(
            plain(&summary(2, 3, elapsed, Some("fixture-id"), true)),
            ">_ jecode closed · 2 prompts · 3 tools · 4m 12s\n   jecode resume fixture-id"
        );
        assert_eq!(
            plain(&summary(1, 1, elapsed, Some("fixture-id"), false)),
            ">_ jecode closed · 1 prompt · 1 tool · 4m 12s · autosave failed"
        );
        assert_eq!(summary(0, 0, elapsed, None, true).lines().count(), 1);
        assert_eq!(
            summary(0, 0, elapsed, Some("unsaved-id"), true)
                .lines()
                .count(),
            1
        );
    }

    #[test]
    fn exit_colours_use_only_bold_dim_and_magenta() {
        let text = summary(2, 3, Duration::from_secs(5), Some("fixture-id"), true);
        let mut codes: Vec<_> = text
            .split('\x1b')
            .skip(1)
            .map(|part| &part[..=part.find('m').unwrap()])
            .collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes, ["[0m", "[1m", "[2m", "[35m"]);
        assert!(text.lines().nth(1).unwrap().find('\x1b').is_none());
    }
}
