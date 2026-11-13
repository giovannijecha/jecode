//! The one command table. Parsing, contextual help and shell completions all
//! read it, so a command or option exists in all three or in none.

mod commands;
use commands::COMMANDS;

pub struct Opt {
    pub long: &'static str,
    pub short: Option<char>,
    /// The value's placeholder; `None` marks a flag.
    pub value: Option<&'static str>,
    pub help: &'static str,
    pub repeat: bool,
    /// Accepted values, checked before anything runs; empty accepts any.
    pub choices: &'static [&'static str],
}

pub struct Command {
    pub name: &'static str,
    pub summary: &'static str,
    pub usage: &'static [&'static str],
    pub options: &'static [&'static Opt],
    pub subcommands: &'static [Command],
    /// The subcommand a family runs when none is named.
    pub default: Option<&'static str>,
    /// Positional operands as (minimum, maximum).
    pub operands: (usize, usize),
    /// Positionals form a literal prompt, which ends option parsing.
    pub prompt: bool,
    pub details: &'static str,
}

const fn flag(long: &'static str, help: &'static str) -> Opt {
    Opt {
        long,
        short: None,
        value: None,
        help,
        repeat: false,
        choices: &[],
    }
}

const fn valued(long: &'static str, value: &'static str, help: &'static str) -> Opt {
    Opt {
        long,
        short: None,
        value: Some(value),
        help,
        repeat: false,
        choices: &[],
    }
}

pub const PROVIDERS: &[&str] = &[
    "openrouter",
    "openai-api",
    "anthropic-api",
    "openai",
    "anthropic",
];
pub const EFFORTS: &[&str] = &[
    "default", "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];
pub const LEVELS: &[&str] = &["read", "workspace", "full"];
pub const SHELLS: &[&str] = &["bash", "zsh", "fish", "powershell"];

pub static HELP: Opt = Opt {
    short: Some('h'),
    ..flag("help", "Show help for this command; never runs it")
};
pub static VERSION: Opt = Opt {
    short: Some('V'),
    ..flag("version", "Print the version and build identity")
};
pub static CD: Opt = Opt {
    short: Some('C'),
    ..valued("cd", "DIR", "Work in DIR instead of the current directory")
};
pub static PLAIN: Opt = flag("plain", "Use the line-based chat instead of the TUI");
pub static PROVIDER: Opt = Opt {
    choices: PROVIDERS,
    ..valued(
        "provider",
        "ID",
        "Provider for this run: openrouter, openai-api, anthropic-api (API keys), openai, anthropic (accounts)",
    )
};
pub static MODEL: Opt = valued("model", "ID", "Model for this run");
pub static ROUTING: Opt = valued(
    "routing",
    "auto|ENDPOINT",
    "OpenRouter routing for this run",
);
pub static EFFORT: Opt = Opt {
    choices: EFFORTS,
    ..valued("effort", "NAME", "Reasoning effort for this run")
};
pub static PERMISSIONS: Opt = Opt {
    choices: LEVELS,
    ..valued(
        "permissions",
        "LEVEL",
        "Permission level for this run: read, workspace or full",
    )
};
pub static ATTACH: Opt = Opt {
    repeat: true,
    ..valued("attach", "PATH", "Attach a file to the first message")
};
pub static JSON: Opt = flag(
    "json",
    "Structured output: one JSON document, or JSON Lines for a run",
);
pub static LAST: Opt = flag("last", "Resume the most recent conversation in this folder");
pub static YES: Opt = flag("yes", "Confirm without an interactive prompt");
pub static ACCOUNT: Opt = valued("account", "ACCOUNT_ID", "Remove this saved account");
pub static CHECK: Opt = flag("check", "Only check whether a newer version is published");
pub static OUTPUT: Opt = valued("output", "PATH", "Write to PATH; - writes to stdout");
pub static SETTING_PROVIDER: Opt = Opt {
    choices: PROVIDERS,
    ..valued(
        "provider",
        "ID",
        "Provider of the model (default: the saved provider)",
    )
};
pub static AUTH: Opt = Opt {
    choices: &["none", "token", "oauth"],
    ..valued(
        "auth",
        "METHOD",
        "Sign-in method: none, token (read from stdin or a hidden prompt) or oauth",
    )
};
pub static EXECUTABLE: Opt = valued(
    "executable",
    "PATH",
    "Server executable or command name (default: the preset's command)",
);

const RUN: &[&Opt] = &[
    &PLAIN,
    &PROVIDER,
    &MODEL,
    &ROUTING,
    &EFFORT,
    &PERMISSIONS,
    &ATTACH,
    &JSON,
];
const RESUME: &[&Opt] = &[
    &LAST,
    &PLAIN,
    &PROVIDER,
    &MODEL,
    &ROUTING,
    &EFFORT,
    &PERMISSIONS,
    &ATTACH,
    &JSON,
];
const INIT: &[&Opt] = &[&PROVIDER, &MODEL, &ROUTING, &EFFORT, &PERMISSIONS, &JSON];
const JSON_ONLY: &[&Opt] = &[&JSON];
const NONE: &[&Opt] = &[];

/// Options valid with every command.
pub const GLOBAL: &[&Opt] = &[&HELP, &CD];

const fn leaf(
    name: &'static str,
    summary: &'static str,
    usage: &'static [&'static str],
    options: &'static [&'static Opt],
    operands: (usize, usize),
) -> Command {
    Command {
        name,
        summary,
        usage,
        options,
        subcommands: &[],
        default: None,
        operands,
        prompt: false,
        details: "",
    }
}

pub static ROOT: Command = Command {
    name: "jecode",
    summary: "A terminal coding agent",
    usage: &[
        "[OPTIONS] [--] [PROMPT]",
        "[OPTIONS] resume [--last | SESSION_ID] [--] [PROMPT]",
        "COMMAND [ARGS] [OPTIONS]",
    ],
    options: RUN,
    subcommands: &COMMANDS,
    default: None,
    operands: (0, usize::MAX),
    prompt: true,
    details: "Without PROMPT, Jecode opens the TUI in an interactive terminal, or runs the task\nread from piped stdin. With PROMPT it runs one task and exits; piped stdin is\nadded to the task as data. Answers go to stdout, progress to stderr.\nRun options never change saved defaults. Put -- before a prompt that starts\nwith '-' or with a command name, or quote the whole prompt.",
};

/// Every option once, for value handling before the command is known.
pub fn option_named(long: &str) -> Option<&'static Opt> {
    known().into_iter().find(|option| option.long == long)
}

pub fn option_short(short: char) -> Option<&'static Opt> {
    known()
        .into_iter()
        .find(|option| option.short == Some(short))
}

fn known() -> Vec<&'static Opt> {
    let mut all: Vec<&'static Opt> = vec![&VERSION];
    all.extend(GLOBAL.iter().copied());
    fn visit(command: &'static Command, all: &mut Vec<&'static Opt>) {
        for option in command.options {
            if !all.iter().any(|known| known.long == option.long) {
                all.push(option);
            }
        }
        for sub in command.subcommands {
            visit(sub, all);
        }
    }
    visit(&ROOT, &mut all);
    all
}

impl Command {
    pub fn subcommand(&self, name: &str) -> Option<&Command> {
        self.subcommands.iter().find(|command| command.name == name)
    }

    pub fn allows(&self, long: &str) -> bool {
        GLOBAL.iter().any(|option| option.long == long)
            || self.options.iter().any(|option| option.long == long)
    }
}

/// The command at `path` below the root.
pub fn command(path: &[&str]) -> Option<&'static Command> {
    let mut command = &ROOT;
    for name in path {
        command = command.subcommand(name)?;
    }
    Some(command)
}
