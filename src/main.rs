use std::{
    env,
    ffi::OsStr,
    io::{self, Write},
    process::ExitCode,
};

const HELP: &str = "Jecode — owned coding harness\n\nUsage: jecode [--workspace PATH] [--model MODEL] [--access local|workspace]\n       jecode resume [SESSION_ID]\n       jecode sessions | chat | logout\n\n  jecode        Start in the current directory with your saved account\n  resume        Choose a saved conversation, or reopen SESSION_ID\n  sessions      List recent conversations\n  chat          Start a conversation without file tools\n  logout        Remove saved account access\n\n  --workspace PATH  Use another working directory\n  --model MODEL     gpt-5.6-luna (default) or gpt-5.6-terra; medium effort\n  --access PROFILE  local (default) or workspace\n  --demo            Offline terminal preview\n  -h, --help        Show this help\n  -V, --version     Show the native version\n\nInside Jecode, type / for commands. Use arrows and Enter to choose.\nFile changes and commands require approval. Commands run with your user permissions.\nCredentials, settings and sessions use ordinary JSON in ~/.jecode/v1/.\nResume restores saved access and never replays historical tools.\nLegacy --account, --resume, --sessions and --logout remain supported.\n";

fn run(mut args: impl Iterator<Item = std::ffi::OsString>) -> io::Result<u8> {
    let first = args.next();
    if matches!(
        first.as_deref().and_then(OsStr::to_str),
        Some("resume" | "--resume")
    ) {
        let id = match args.next() {
            None => match jecode::terminal::sessions(true) {
                Ok(Some(id)) => Some(id),
                Ok(None) => return Ok(0),
                Err(error) => {
                    return terminal_result(
                        Err(error),
                        "cannot list saved sessions; check ~/.jecode/v1/sessions",
                    );
                }
            },
            Some(id) => id.into_string().ok(),
        };
        if let Some(id) = id
            && args.next().is_none()
        {
            return terminal_result(
                jecode::terminal::resume(&id),
                "cannot resume; use jecode sessions, close any other owner, and check the saved workspace and ~/.jecode/v1/settings.json",
            );
        }
        writeln!(
            io::stderr().lock(),
            "jecode: expected a saved session ID; use jecode resume to choose"
        )?;
        return Ok(2);
    }
    let entry = first.as_deref().and_then(OsStr::to_str);
    let legacy = entry == Some("--account");
    let chat = entry == Some("chat");
    if first.is_none()
        || legacy
        || chat
        || matches!(entry, Some("--workspace" | "--model" | "--access"))
    {
        let options = first.filter(|_| !legacy && !chat).into_iter().chain(args);
        if let Some(AccountOptions {
            model,
            workspace: path,
            access,
        }) = account_options(options, !legacy && !chat)
        {
            if chat && (path.is_some() || access.is_some()) {
                writeln!(
                    io::stderr().lock(),
                    "jecode: chat does not use file tools; run jecode to work in a directory"
                )?;
                return Ok(2);
            }
            let workspace = match path {
                None => None,
                Some(path) => match jecode::workspace::Workspace::open(std::path::Path::new(&path))
                {
                    Ok(workspace) => Some(workspace),
                    Err(_) => {
                        writeln!(
                            io::stderr().lock(),
                            "jecode: cannot open the selected workspace; use a supported local directory"
                        )?;
                        return Ok(2);
                    }
                },
            };
            return terminal_result(
                jecode::terminal::configured_account(model, workspace, access),
                "cannot start a session; check ~/.jecode/v1/settings.json and private user-directory permissions",
            );
        }
        writeln!(
            io::stderr().lock(),
            "jecode: invalid start options; use jecode --help"
        )?;
        return Ok(2);
    }
    if args.next().is_some() {
        writeln!(
            io::stderr().lock(),
            "jecode: expected one option; use --help"
        )?;
        return Ok(2);
    }
    match first.as_deref() {
        Some(arg) if arg == OsStr::new("--sessions") || arg == OsStr::new("sessions") => {
            match jecode::terminal::sessions(false) {
                Ok(_) => Ok(0),
                Err(_) => {
                    writeln!(
                        io::stderr().lock(),
                        "jecode: cannot list saved sessions; check ~/.jecode/v1/sessions"
                    )?;
                    Ok(2)
                }
            }
        }
        Some(arg) if arg == OsStr::new("--logout") || arg == OsStr::new("logout") => {
            let cancelled = std::sync::atomic::AtomicBool::new(false);
            let budget = jecode::tls::Budget {
                cancelled: &cancelled,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(5),
            };
            match jecode::providers::openai_account::client::Client::logout(&budget) {
                Ok(()) => {
                    writeln!(io::stdout().lock(), "Signed out of Jecode.")?;
                    Ok(0)
                }
                Err(error) => {
                    writeln!(io::stderr().lock(), "jecode: {error}")?;
                    Ok(2)
                }
            }
        }
        Some(arg) if arg == OsStr::new("--demo") => terminal_result(
            jecode::terminal::demo(),
            "terminal initialization or I/O failed",
        ),
        Some(arg) if arg == OsStr::new("--help") || arg == OsStr::new("-h") => {
            io::stdout().lock().write_all(HELP.as_bytes())?;
            Ok(0)
        }
        Some(arg) if arg == OsStr::new("--version") || arg == OsStr::new("-V") => {
            writeln!(io::stdout().lock(), "jecode {}", jecode::VERSION)?;
            Ok(0)
        }
        _ => {
            // Do not echo arbitrary arguments, which may contain secrets or terminal escapes.
            writeln!(io::stderr().lock(), "jecode: unknown option; use --help")?;
            Ok(2)
        }
    }
}

struct AccountOptions {
    model: Option<jecode::session::Model>,
    workspace: Option<std::ffi::OsString>,
    access: Option<jecode::workspace::Access>,
}
fn account_options(
    mut args: impl Iterator<Item = std::ffi::OsString>,
    current_directory: bool,
) -> Option<AccountOptions> {
    let mut model = None;
    let mut workspace = None;
    let mut access = None;
    while let Some(option) = args.next() {
        if option == "--model" && model.is_none() {
            model = Some(match args.next()?.to_str()? {
                "gpt-5.6-luna" => jecode::session::Model::Luna,
                "gpt-5.6-terra" => jecode::session::Model::Terra,
                _ => return None,
            });
        } else if option == "--access" && access.is_none() {
            access = Some(jecode::workspace::Access::parse(args.next()?.to_str()?)?);
        } else if option == "--workspace" && workspace.is_none() {
            let path = args.next()?;
            if path.is_empty() {
                return None;
            }
            workspace = Some(path);
        } else {
            return None;
        }
    }
    if current_directory && workspace.is_none() {
        workspace = Some(".".into());
    }
    if access.is_some() && workspace.is_none() {
        return None;
    }
    Some(AccountOptions {
        model,
        workspace,
        access,
    })
}

fn terminal_result(result: io::Result<()>, context: &str) -> io::Result<u8> {
    match result {
        Ok(()) => Ok(0),
        Err(_) => {
            use io::IsTerminal;
            let message = if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                "requires a supported interactive terminal"
            } else {
                context
            };
            // Startup diagnostics are fixed text, never session contents or tokens.
            writeln!(io::stderr().lock(), "jecode: {message}")?;
            Ok(2)
        }
    }
}

fn main() -> ExitCode {
    match run(env::args_os().skip(1)) {
        Ok(code) => ExitCode::from(code),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn start_defaults_to_current_directory_but_legacy_chat_stays_explicit() {
        let parse =
            |args: &[&str], cwd| account_options(args.iter().map(std::ffi::OsString::from), cwd);
        assert_eq!(
            parse(&[], true).unwrap().workspace.as_deref(),
            Some(OsStr::new("."))
        );
        assert!(parse(&[], false).unwrap().workspace.is_none());
        assert!(parse(&["--access", "local"], true).is_some());
        assert!(parse(&["--access", "local"], false).is_none());
        assert_eq!(
            parse(&["--workspace", "sibling"], true)
                .unwrap()
                .workspace
                .as_deref(),
            Some(OsStr::new("sibling"))
        );
        assert!(parse(&["--model", "unknown"], true).is_none());
    }
}
