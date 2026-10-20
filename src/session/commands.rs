pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
    pub tui_usage: &'static str,
    pub plain_usage: Option<&'static str>,
}

impl Command {
    pub fn description(&self, tui: bool) -> &'static str {
        match (tui, self.name) {
            (false, "/attach") => "Attach files to the next message",
            (false, "/terminal") => "List active processes, or show saved output by ID",
            (false, "/help") => "Show commands",
            (false, "/exit") => "Quit",
            _ => self.description,
        }
    }
}

pub const COMMANDS: [Command; 15] = [
    Command {
        name: "/new",
        description: "Start a new conversation",
        tui_usage: "/new",
        plain_usage: Some("/new"),
    },
    Command {
        name: "/resume",
        description: "Resume or delete a conversation in this folder",
        tui_usage: "/resume [ID]",
        plain_usage: Some("/resume [ID]"),
    },
    Command {
        name: "/model",
        description: "Choose the model for this conversation",
        tui_usage: "/model [ID]",
        plain_usage: Some("/model [ID]"),
    },
    Command {
        name: "/effort",
        description: "Choose the reasoning effort for this conversation",
        tui_usage: "/effort [NAME]",
        plain_usage: Some("/effort [NAME]"),
    },
    Command {
        name: "/routing",
        description: "Choose how OpenRouter routes this conversation to endpoints",
        tui_usage: "/routing",
        plain_usage: Some("/routing"),
    },
    Command {
        name: "/settings",
        description: "Change saved defaults and the OpenRouter key",
        tui_usage: "/settings",
        plain_usage: Some("/settings"),
    },
    Command {
        name: "/init",
        description: "Inspect this project and create or update root JECODE.md",
        tui_usage: "/init",
        plain_usage: Some("/init"),
    },
    Command {
        name: "/help",
        description: "Show commands and keyboard controls",
        tui_usage: "/help",
        plain_usage: Some("/help"),
    },
    Command {
        name: "/export",
        description: "Save the current conversation as JSON",
        tui_usage: "/export",
        plain_usage: Some("/export"),
    },
    Command {
        name: "/copy",
        description: "Copy the last completed response or one of its blocks",
        tui_usage: "/copy",
        plain_usage: Some("/copy"),
    },
    Command {
        name: "/attach",
        description: "Attach files to the draft; drop files or press Alt+V for an image",
        tui_usage: "/attach PATH",
        plain_usage: Some("/attach PATH"),
    },
    Command {
        name: "/tmp",
        description: "Show session temporary files; /tmp clean clears them",
        tui_usage: "/tmp [clean]",
        plain_usage: Some("/tmp [clean]"),
    },
    Command {
        name: "/terminal",
        description: "Inspect or stop this conversation's background processes",
        tui_usage: "/terminal [ID]",
        plain_usage: Some("/terminal [ID]"),
    },
    Command {
        name: "/drafts",
        description: "Edit or discard pending drafts in the terminal interface",
        tui_usage: "/drafts",
        plain_usage: None,
    },
    Command {
        name: "/exit",
        description: "Quit after pending work completes",
        tui_usage: "/exit",
        plain_usage: Some("/exit"),
    },
];

const TUI_CONTROLS: &str = "Enter         Send, or queue while working\nCtrl+J        New line (Shift/Alt+Enter when supported)\nAlt+V         Attach the clipboard image\nBackspace/Del Remove an attachment element whole\nAlt+Up        Open pending drafts (also /drafts)\nCtrl+P/N      Sent prompt history, excluding commands\nEsc           Close a panel, return to bottom, or stop work\nCtrl+C        Stop work, close a menu, or clear the draft; never quit\nCtrl+Q        Cancel active work and quit\nCtrl+D        In /terminal, mark stop; in /resume or /drafts, mark deletion; Enter confirms, Esc cancels\nCtrl+S        Send or queue a paused draft in /drafts\n\nPage Up/Down and mouse wheel browse the conversation.\nAlt+End or Esc while reading returns to the bottom.\nHost scrolling shortcuts take precedence.";
const PLAIN_CONTROLS: &str = "/attach --clear Discard staged files without sending\n/terminal stop ID Request a stop for a background process\n\nType a message and press Enter to send it. Type /exit to quit.\nIn /resume, type d NUMBER or d ID to delete a listed conversation.";

pub fn help(tui: bool) -> String {
    let mut rows = COMMANDS
        .iter()
        .filter_map(|command| {
            let usage = if tui {
                Some(command.tui_usage)
            } else {
                command.plain_usage
            }?;
            Some(format!("{usage:<17}{}", command.description(tui)))
        })
        .collect::<Vec<_>>();
    rows.push(String::new());
    rows.push(if tui { TUI_CONTROLS } else { PLAIN_CONTROLS }.into());
    rows.join("\n")
}

pub fn split(text: &str) -> Option<(&str, &str)> {
    let text = text.trim();
    let (name, arguments) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    name.starts_with('/').then_some((name, arguments.trim()))
}

pub fn canonical(name: &str) -> &str {
    if name.eq_ignore_ascii_case("/quit") {
        "/exit"
    } else if name.eq_ignore_ascii_case("/clear") {
        "/new"
    } else if name.eq_ignore_ascii_case("/setup") {
        "/settings"
    } else {
        name
    }
}

pub const INIT_PROMPT: &str = "/init\n\nInspect this project's files and conventions using the available tools. Create or update only the root JECODE.md with concise, verified instructions for future Jecode turns. Preserve useful existing content and make targeted updates instead of replacing it wholesale. Base every rule on what you observe in this project; do not invent commands or policies. Do not write other files.";
/// Returns the ID after a local `/terminal stop ID` action, including an empty
/// ID when `stop` was supplied without an argument.
pub fn terminal_stop_id(arguments: &str) -> Option<&str> {
    let mut parts = arguments.trim().splitn(2, char::is_whitespace);
    parts
        .next()
        .filter(|action| action.eq_ignore_ascii_case("stop"))
        .map(|_| parts.next().unwrap_or("").trim())
}

#[derive(Clone, Debug)]
pub struct Match {
    pub index: usize,
    pub positions: Vec<usize>,
    score: usize,
}
pub fn match_text(query: &str, candidate: &str) -> Option<(usize, Vec<usize>)> {
    let query: Vec<char> = query.to_lowercase().chars().collect();
    let candidate: Vec<char> = candidate.to_lowercase().chars().collect();
    if query.is_empty() {
        return Some((0, vec![]));
    }
    if let Some(start) = candidate
        .windows(query.len())
        .position(|window| window == query)
    {
        return Some((start, (start..start + query.len()).collect()));
    }
    let mut positions = Vec::new();
    let mut cursor = 0;
    for character in query {
        let offset = candidate[cursor..]
            .iter()
            .position(|value| *value == character)?;
        cursor += offset;
        positions.push(cursor);
        cursor += 1;
    }
    let score =
        1000 + positions.last().copied().unwrap_or(0) - positions.first().copied().unwrap_or(0);
    Some((score, positions))
}
pub fn suggestions(query: &str) -> Vec<Match> {
    let mut matches: Vec<_> = COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(index, command)| {
            match_text(query, command.name).map(|(score, positions)| Match {
                index,
                score,
                positions,
            })
        })
        .collect();
    matches.sort_by_key(|matched| (matched.score, matched.index));
    matches
}

pub fn unknown(name: &str) -> String {
    let correction = COMMANDS
        .iter()
        .map(|command| (distance(name, command.name), command.name))
        .min();
    match correction {
        Some((distance, correction)) if distance <= 2 => {
            format!("Unknown command: {name}. Did you mean {correction}?")
        }
        _ => format!("Unknown command: {name}. Use /help to see commands."),
    }
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<_> = b.to_lowercase().chars().collect();
    let mut row: Vec<_> = (0..=b.len()).collect();
    for (i, a) in a.to_lowercase().chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, b) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (previous + usize::from(a != *b))
                .min(row[j] + 1)
                .min(old + 1);
            previous = old;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contiguous_matches_rank_first_and_typos_get_a_local_correction() {
        let matches = suggestions("/MO");
        assert_eq!(COMMANDS[matches[0].index].name, "/model");
        assert_eq!(matches[0].positions, [0, 1, 2]);
        assert_eq!(COMMANDS[suggestions("/mdl")[0].index].name, "/model");
        assert!(suggestions("/nonsense").is_empty());
        assert!(unknown("/modle").contains("/model"));
    }

    #[test]
    fn terminal_stop_parser_preserves_the_remaining_id() {
        assert_eq!(terminal_stop_id("stop process-1"), Some("process-1"));
        assert_eq!(terminal_stop_id(" STOP \t process-1  "), Some("process-1"));
        assert_eq!(terminal_stop_id("stop"), Some(""));
        assert_eq!(terminal_stop_id("stop  "), Some(""));
        assert_eq!(terminal_stop_id("stopping process-1"), None);
        assert_eq!(terminal_stop_id("process-1"), None);
    }

    #[test]
    fn visible_commands_share_catalog_and_help_without_hidden_aliases() {
        for command in COMMANDS.iter() {
            assert!(help(true).contains(command.tui_usage), "{}", command.name);
            if let Some(usage) = command.plain_usage {
                assert!(help(false).contains(usage), "{}", command.name);
            }
            assert!(
                suggestions(command.name)
                    .iter()
                    .any(|entry| COMMANDS[entry.index].name == command.name)
            );
        }
        for alias in ["/quit", "/clear", "/setup"] {
            assert!(!COMMANDS.iter().any(|command| command.name == alias));
            assert!(!help(true).contains(alias));
            assert!(!help(false).contains(alias));
        }
        assert_eq!(canonical("/QUIT"), "/exit");
        assert_eq!(canonical("/CLEAR"), "/new");
        assert_eq!(canonical("/SETUP"), "/settings");
        assert_eq!(split("  /InIt \t "), Some(("/InIt", "")));
        assert!(help(false).contains("Attach files to the next message"));
        assert!(!help(false).contains("Alt+V"));
        assert!(help(true).contains("Alt+V"));
    }
}
