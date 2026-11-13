//! What a parsed command line asks for: the command's action, the run it
//! starts, or the help it shows.

use super::*;

pub(super) fn action(
    command: &'static Command,
    path: &[&'static str],
    positionals: Vec<String>,
    values: Vec<(&'static str, Option<String>)>,
) -> Result<Action, Usage> {
    let flag = |long: &str| values.iter().any(|(name, _)| *name == long);
    if command.prompt || command.name == "init" {
        return run(command, path, positionals, &values).map(Action::Run);
    }
    let (minimum, maximum) = command.operands;
    if positionals.len() < minimum || positionals.len() > maximum {
        let detail = match (minimum, maximum) {
            (0, 0) => "takes no arguments".to_owned(),
            (low, high) if low == high => format!("takes {low} argument{}", plural(low)),
            (low, high) => format!("takes {low} to {high} arguments"),
        };
        return Err(Usage::new(
            format!(
                "'jecode {}' {detail}; got {}{}",
                path.join(" "),
                positionals.len(),
                if path.len() == 1 && positionals.len() > maximum {
                    ". Quote a prompt that starts with a command name, or put -- before it"
                } else {
                    ""
                }
            ),
            path,
        ));
    }
    Ok(match path {
        ["setup"] => Action::Setup,
        ["doctor"] => Action::Doctor { json: flag("json") },
        ["update"] => Action::Update {
            check: flag("check"),
            json: flag("json"),
        },
        ["uninstall"] => Action::Uninstall {
            yes: flag("yes"),
            json: flag("json"),
        },
        ["completions"] => {
            let shell = positionals.into_iter().next().unwrap();
            if !grammar::SHELLS.contains(&shell.as_str()) {
                return Err(Usage::new(
                    format!(
                        "unsupported shell '{shell}'; use {}",
                        grammar::SHELLS.join(", ")
                    ),
                    path,
                ));
            }
            Action::Completions(shell)
        }
        ["help"] => {
            let names: Vec<&str> = positionals.iter().map(String::as_str).collect();
            let mut target = Vec::new();
            let mut current: &'static Command = &grammar::ROOT;
            for name in names {
                let next = current
                    .subcommand(name)
                    .filter(|_| name != "help")
                    .ok_or_else(|| {
                        Usage::new(format!("no help for unknown command '{name}'"), path)
                    })?;
                target.push(next.name);
                current = next;
            }
            Action::Help(target)
        }
        [family, sub] => Action::Manage(Manage {
            family,
            action: sub,
            operands: positionals,
            values,
        }),
        _ => unreachable!("every leaf command is handled"),
    })
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn run(
    command: &Command,
    path: &[&'static str],
    mut positionals: Vec<String>,
    values: &[(&'static str, Option<String>)],
) -> Result<Run, Usage> {
    let value = |long: &str| {
        values
            .iter()
            .find(|(name, _)| *name == long)
            .and_then(|(_, value)| value.clone())
    };
    let flag = |long: &str| values.iter().any(|(name, _)| *name == long);
    let start = match command.name {
        "init" => {
            if !positionals.is_empty() {
                return Err(Usage::new(
                    "'jecode init' takes no prompt; it runs the /init task",
                    path,
                ));
            }
            Start::Init
        }
        "resume" if flag("last") => Start::Resume(Resume::Last),
        "resume" if positionals.is_empty() => Start::Resume(Resume::Pick),
        "resume" => {
            let id = positionals.remove(0);
            if !crate::sessions::valid_id(&id) {
                return Err(Usage::new(
                    format!(
                        "'{id}' is not a session ID. Use jecode sessions list to find one, or --last for the most recent"
                    ),
                    path,
                ));
            }
            Start::Resume(Resume::Id(id))
        }
        _ => Start::New,
    };
    let prompt = (!positionals.is_empty()).then(|| positionals.join(" "));
    if prompt.as_ref().is_some_and(|text| text.trim().is_empty()) {
        return Err(Usage::new("the prompt must not be empty", path));
    }
    if let Some(model) = value("model") {
        crate::config::validate_model(&model).map_err(|error| Usage::new(error, path))?;
    }
    let routing = value("routing")
        .map(|value| {
            if value == "auto" {
                Ok(crate::backend::Routing::Auto)
            } else if value.len() <= 128 && value.bytes().all(|byte| byte.is_ascii_graphic()) {
                Ok(crate::backend::Routing::Endpoint(value))
            } else {
                Err(Usage::new(
                    "--routing needs auto or an endpoint tag of at most 128 ASCII characters",
                    path,
                ))
            }
        })
        .transpose()?;
    let attachments: Vec<String> = values
        .iter()
        .filter(|(name, _)| *name == "attach")
        .filter_map(|(_, value)| value.clone())
        .collect();
    if start == Start::Resume(Resume::Pick) && (flag("json") || prompt.is_some()) {
        return Err(Usage::new(
            "choosing a conversation is interactive; supply SESSION_ID or --last",
            path,
        ));
    }
    if flag("json") && flag("plain") {
        return Err(Usage::new(
            "--json and --plain cannot be combined: --json is for single runs",
            path,
        ));
    }
    Ok(Run {
        start,
        prompt,
        attachments,
        plain: flag("plain"),
        json: flag("json"),
        selection: Selection {
            provider: value("provider"),
            model: value("model"),
            routing,
            effort: value("effort").map(|name| Effort::parse(&name).expect("checked choice")),
            permissions: value("permissions")
                .map(|name| Level::parse(&name).expect("checked choice")),
        },
    })
}

/// `-h`/`--help` before `--` asks for the help of the deepest named command.
pub(super) fn help_request(tokens: &[String]) -> Result<Option<Vec<&'static str>>, Usage> {
    let end = tokens
        .iter()
        .position(|token| token == "--")
        .unwrap_or(tokens.len());
    if !tokens[..end]
        .iter()
        .any(|token| token == "-h" || token == "--help")
    {
        return Ok(None);
    }
    let mut command: &'static Command = &grammar::ROOT;
    let mut path = Vec::new();
    let mut index = 0;
    while index < end {
        let token = &tokens[index];
        index += 1;
        if token.starts_with('-') && token.len() > 1 {
            let takes_value = if let Some(long) = token.strip_prefix("--") {
                !long.contains('=')
                    && grammar::option_named(long).is_some_and(|option| option.value.is_some())
            } else {
                token[1..]
                    .chars()
                    .next()
                    .and_then(grammar::option_short)
                    .is_some_and(|option| option.value.is_some())
            };
            if takes_value {
                index += 1;
            }
            continue;
        }
        match command.subcommand(token) {
            Some(sub) => {
                command = sub;
                path.push(sub.name);
            }
            None => break,
        }
    }
    Ok(Some(path))
}
