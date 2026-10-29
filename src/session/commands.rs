pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
    pub tui_usage: &'static str,
    pub plain_usage: Option<&'static str>,
}

/// Command groups in the order menus list them.
pub const GROUPS: [&str; 4] = ["Conversation", "Model", "Workspace", "Jecode"];

impl Command {
    pub fn group(&self) -> &'static str {
        match self.name {
            "/new" | "/resume" | "/copy" | "/export" | "/drafts" | "/agents" | "/ask" | "/todo"
            | "/exit" => GROUPS[0],
            "/model" | "/effort" | "/routing" => GROUPS[1],
            "/settings" | "/help" => GROUPS[3],
            _ => GROUPS[2],
        }
    }
    pub fn description(&self, tui: bool) -> &'static str {
        match (tui, self.name) {
            (false, "/terminal") => "List processes, inspect output or send local input",
            (false, "/help") => "Show commands",
            (false, "/exit") => "Quit",
            _ => self.description,
        }
    }
}

pub const COMMANDS: [Command; 19] = [
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
        description: "Choose endpoint routing when the backend supports it",
        tui_usage: "/routing",
        plain_usage: Some("/routing"),
    },
    Command {
        name: "/settings",
        description: "Change saved defaults and account credentials",
        tui_usage: "/settings",
        plain_usage: Some("/settings"),
    },
    Command {
        name: "/init",
        description: "Inspect this project and create or update root AGENTS.md",
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
        name: "/skills",
        description: "List available personal skills",
        tui_usage: "/skills",
        plain_usage: Some("/skills"),
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
        name: "/tmp",
        description: "Show session temporary files; /tmp clean clears them",
        tui_usage: "/tmp [clean]",
        plain_usage: Some("/tmp [clean]"),
    },
    Command {
        name: "/checkpoints",
        description: "Inspect, restore or configure workspace checkpoints; /checkpoints help",
        tui_usage: "/checkpoints",
        plain_usage: Some("/checkpoints"),
    },
    Command {
        name: "/terminal",
        description: "Inspect, stop or type into a background process",
        tui_usage: "/terminal [ID]",
        plain_usage: Some("/terminal [ID]"),
    },
    Command {
        name: "/agents",
        description: "Inspect collaborators in this conversation",
        tui_usage: "/agents",
        plain_usage: Some("/agents"),
    },
    Command {
        name: "/ask",
        description: "Return to the pending question",
        tui_usage: "/ask",
        plain_usage: None,
    },
    Command {
        name: "/todo",
        description: "Inspect this conversation's current plan",
        tui_usage: "/todo",
        plain_usage: Some("/todo"),
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

const TUI_CONTROLS: &str = "Enter         Send, or queue while working\nCtrl+J        New line (Shift/Alt+Enter when supported)\nAlt+V         Attach the clipboard image\nBackspace/Del Remove an attachment element whole\nAlt+Up        Open pending drafts (also /drafts)\nCtrl+P/N      Sent prompt history, excluding commands\nEsc           Close a panel, return to bottom, or stop work\nCtrl+C        Stop work, close a menu, or clear the draft; never quit\nCtrl+Q        Quit, asking first while work runs\nCtrl+D        In /terminal, mark stop; in /resume or /drafts, mark deletion; Enter confirms, Esc cancels\nCtrl+S        Send or queue a paused draft in /drafts\ni             In active terminal output, enter direct input mode\nCtrl+]       Leave direct input; Ctrl+C sends an interrupt there\n\nPage Up/Down and mouse wheel browse the conversation.\nAlt+End or Esc while reading returns to the bottom.\nHost scrolling shortcuts take precedence.";
const PLAIN_CONTROLS: &str = "/terminal write ID TEXT   Send one line directly to an interactive process\n/terminal interrupt ID    Send Ctrl+C to an interactive process\n/terminal resize ID C R   Resize its terminal to C columns and R rows\n/terminal stop ID         Request a stop for a background process\n\nUse fullscreen direct input for hidden password entry.\nType a message and press Enter to send it. Type /exit to quit.\nIn /resume, type d NUMBER or d ID to delete a listed conversation.";

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

pub fn skills_catalog(
    skills: impl IntoIterator<Item = (String, String)>,
    warnings: &[String],
) -> (String, Vec<(String, String)>) {
    let mut details: Vec<_> = skills.into_iter().collect();
    let count = details.len();
    let result = match (count, warnings.len()) {
        (0, 0) => "No skills installed.".to_owned(),
        (0, skipped) => format!("No valid skills found · {skipped} skipped"),
        (count, 0) => format!(
            "{count} available {}",
            if count == 1 { "skill" } else { "skills" }
        ),
        (count, skipped) => format!(
            "{count} available {} · {skipped} skipped",
            if count == 1 { "skill" } else { "skills" }
        ),
    };
    details.extend(
        warnings
            .iter()
            .cloned()
            .map(|warning| ("Skipped".into(), warning)),
    );
    (result, details)
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

pub const INIT_PROMPT: &str = "/init\n\nInspect this project's files and conventions using the available tools. Create or update only the root AGENTS.md with concise, verified project instructions for coding agents. Preserve useful existing content and make targeted updates instead of replacing it wholesale. Base every rule on what you observe in this project; do not invent commands or policies. Do not write other files.";
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
    // The bare slash lists every command under its group.
    let grouped = |matched: &Match| {
        let group = COMMANDS[matched.index].group();
        if query == "/" {
            GROUPS.iter().position(|name| *name == group).unwrap_or(0)
        } else {
            0
        }
    };
    matches.sort_by_key(|matched| (matched.score, grouped(matched), matched.index));
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
        assert!(!help(false).contains("Alt+V"));
        assert!(help(true).contains("Alt+V"));
        assert!(help(true).contains("/skills"));
        assert!(help(false).contains("/skills"));
    }

    #[test]
    fn skills_catalog_explains_empty_valid_and_skipped_entries() {
        assert_eq!(
            skills_catalog([], &[]),
            ("No skills installed.".into(), vec![])
        );
        assert_eq!(
            skills_catalog([], &["invalid metadata".into()]),
            (
                "No valid skills found · 1 skipped".into(),
                vec![("Skipped".into(), "invalid metadata".into())]
            )
        );
        assert_eq!(
            skills_catalog(
                [("review".into(), "Inspect a change".into())],
                &["broken skill".into()]
            ),
            (
                "1 available skill · 1 skipped".into(),
                vec![
                    ("review".into(), "Inspect a change".into()),
                    ("Skipped".into(), "broken skill".into())
                ]
            )
        );
    }
}
