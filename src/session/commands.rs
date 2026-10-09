pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: [Command; 11] = [
    Command {
        name: "/new",
        description: "Start a new conversation",
    },
    Command {
        name: "/resume",
        description: "Resume or delete a conversation in this folder",
    },
    Command {
        name: "/model",
        description: "Choose the model for this conversation",
    },
    Command {
        name: "/effort",
        description: "Choose the reasoning effort for this conversation",
    },
    Command {
        name: "/settings",
        description: "Change saved defaults and the OpenRouter key",
    },
    Command {
        name: "/help",
        description: "Show commands and keyboard controls",
    },
    Command {
        name: "/export",
        description: "Save the current conversation as JSON",
    },
    Command {
        name: "/copy",
        description: "Copy the last completed response or one of its blocks",
    },
    Command {
        name: "/attach",
        description: "Attach files to the draft; drop files or press Alt+V for an image",
    },
    Command {
        name: "/tmp",
        description: "Show session temporary files; /tmp clean clears them",
    },
    Command {
        name: "/drafts",
        description: "Edit or discard pending drafts in the terminal interface",
    },
];

pub const HELP: &str = "/new          Start a new conversation\n/resume [ID]  Resume or delete a conversation in this folder\n/model [ID]   Choose the model for this conversation\n/effort [NAME] Choose the reasoning effort\n/settings     Change saved defaults and the key\n/help         Show commands and keyboard controls\n/copy         Choose response, code or quote to copy\n/attach PATH  Attach files to the draft; dropped files attach too\n/export       Save this conversation as JSON in the working directory\n/tmp [clean]  Show temporary files, or explicitly clear this session's area\n/drafts       Edit or discard pending drafts in the terminal interface\n/exit         Quit\n\nEnter         Send, or queue while working\nCtrl+J        New line (Shift/Alt+Enter when supported)\nAlt+V         Attach the clipboard image\nBackspace/Del Remove an attachment element whole\nAlt+Up        Open pending drafts (also /drafts)\nCtrl+P/N      Sent prompt history, excluding commands\nEsc           Close a panel, return to bottom, or stop work\nCtrl+C        Stop work, close a menu, or clear the draft; never quit\nCtrl+Q        Cancel active work and quit\nCtrl+D        In /resume or /drafts, mark deletion; Enter confirms, Esc cancels\nCtrl+S        Send or queue a paused draft in /drafts\n\nPage Up/Down and mouse wheel browse the conversation.\nAlt+End or Esc while reading returns to the bottom.\nHost scrolling shortcuts take precedence.";

pub const PLAIN_HELP: &str = "/new          Start a new conversation\n/resume [ID]  Resume or delete a conversation in this folder\n/model [ID]   Choose the model for this conversation\n/effort [NAME] Choose the reasoning effort\n/settings     Change saved defaults and the key\n/help         Show commands\n/copy         Choose response, code or quote to copy\n/attach PATH  Attach files to the next message\n/attach --clear Discard staged files without sending\n/export       Save this conversation as JSON in the working directory\n/tmp [clean]  Show temporary files, or explicitly clear this session's area\n/exit         Quit\n\nType a message and press Enter to send it. Type /exit to quit.\nIn /resume, type d NUMBER or d ID to delete a listed conversation.";
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
}
