//! Help text rendered from the command table. Help never runs a command.

use super::grammar::{self, Command, Opt};

pub fn text(path: &[&str]) -> String {
    let command = grammar::command(path).unwrap_or(&grammar::ROOT);
    if path.is_empty() {
        return root();
    }
    let name = format!("jecode {}", path.join(" "));
    let mut text = format!("{name} — {}\n\nUsage:\n", command.summary);
    for usage in command.usage {
        text.push_str(&format!("  jecode {usage}\n"));
    }
    if !command.details.is_empty() {
        text.push('\n');
        text.push_str(command.details);
        text.push('\n');
    }
    if !command.subcommands.is_empty() {
        text.push_str("\nCommands:\n");
        let width = command
            .subcommands
            .iter()
            .map(|sub| sub.name.len())
            .max()
            .unwrap_or(0);
        for sub in command.subcommands {
            text.push_str(&format!(
                "  {:<width$}  {}{}\n",
                sub.name,
                sub.summary,
                if command.default == Some(sub.name) {
                    " (default)"
                } else {
                    ""
                }
            ));
        }
    }
    let mut options: Vec<&Opt> = command.options.to_vec();
    if options.is_empty() {
        for sub in command.subcommands {
            for option in sub.options {
                if !options.iter().any(|known| known.long == option.long) {
                    options.push(option);
                }
            }
        }
    }
    options.extend(grammar::GLOBAL.iter().copied());
    text.push_str("\nOptions:\n");
    text.push_str(&option_rows(&options));
    text
}

fn root() -> String {
    let root = &grammar::ROOT;
    let mut text = format!(
        "Jecode {} — {}\n\nUsage:\n",
        env!("CARGO_PKG_VERSION"),
        root.summary
    );
    for usage in root.usage {
        text.push_str(&format!("  jecode {usage}\n"));
    }
    text.push('\n');
    text.push_str(root.details);
    text.push_str("\n\nRun options (before the prompt):\n");
    let mut run: Vec<&Opt> = root.options.to_vec();
    run.insert(0, &grammar::CD);
    text.push_str(&option_rows(&run));
    text.push_str("\nCommands:\n");
    let width = root
        .subcommands
        .iter()
        .map(|command| command.name.len())
        .max()
        .unwrap_or(0);
    for command in root.subcommands {
        text.push_str(&format!(
            "  {:<width$}  {}\n",
            command.name, command.summary
        ));
    }
    text.push_str(
        "\nOther options:\n  -h, --help             Show help; with a command, that command's help\n  -V, --version          Print the version and build identity\n",
    );
    text.push_str("\nRun `jecode help COMMAND` or `jecode COMMAND --help` for a command's usage.\nInside Jecode, /help lists slash commands and keys.\nExit status: 0 success, 1 failure, 2 invalid usage.\nJECODE_HOME selects the Jecode home (default ~/.jecode).\n");
    text
}

fn option_rows(options: &[&Opt]) -> String {
    let label = |option: &Opt| {
        let mut label = match option.short {
            Some(short) => format!("-{short}, --{}", option.long),
            None => format!("    --{}", option.long),
        };
        if let Some(value) = option.value {
            label.push(' ');
            label.push_str(value);
        }
        label
    };
    let width = options
        .iter()
        .map(|option| label(option).len())
        .max()
        .unwrap_or(0)
        .max(21);
    options
        .iter()
        .map(|option| {
            format!(
                "  {:<width$}  {}{}\n",
                label(option),
                option.help,
                if option.repeat { " (repeatable)" } else { "" }
            )
        })
        .collect()
}

/// Shell-level commands for discovery inside the TUI and plain chat.
pub fn command_summaries() -> Vec<(String, String)> {
    grammar::ROOT
        .subcommands
        .iter()
        .filter(|command| command.name != "help")
        .map(|command: &Command| (format!("jecode {}", command.name), command.summary.into()))
        .collect()
}
