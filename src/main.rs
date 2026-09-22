use std::{
    env,
    ffi::OsStr,
    io::{self, Write},
    process::ExitCode,
};

const HELP: &str = "Jecode — owned coding harness\n\nUsage: jecode --account [--model MODEL] [--workspace PATH]\n       jecode --resume SESSION_ID\n       jecode --sessions | --logout | --demo | --help | --version\n\n  -h, --help     Show this help\n  -V, --version  Show the native version\n      --demo     Local terminal preview (no model or tools)\n      --account  Stream a new conversation; reuse saved account access\n      --model    gpt-5.6-luna (default) or gpt-5.6-terra; medium effort\n      --workspace  Directory for reads, approved changes and commands\n      --resume   Continue a saved session without replaying tools\n      --sessions List the 50 most recent saved sessions\n      --logout   Remove saved account access from Jecode\n\nCredentials and sessions use ordinary JSON in ~/.jecode/v1/. Without --workspace no files are shared. Commands run with your user permissions after approval.\n";

fn run(mut args: impl Iterator<Item = std::ffi::OsString>) -> io::Result<u8> {
    let first = args.next();
    if first.as_deref() == Some(OsStr::new("--resume")) {
        if let Some(id) = args.next().and_then(|id| id.into_string().ok())
            && args.next().is_none()
        {
            return terminal_result(
                jecode::terminal::resume(&id),
                "cannot resume; use --sessions, close any other owner, and check the saved workspace and ~/.jecode/v1/settings.json",
            );
        }
        writeln!(
            io::stderr().lock(),
            "jecode: expected a saved session ID; use --sessions"
        )?;
        return Ok(2);
    }
    if first.as_deref() == Some(OsStr::new("--account")) {
        if let Some((model, path)) = account_options(args) {
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
                jecode::terminal::configured_account(model, workspace),
                "cannot start a session; check ~/.jecode/v1/settings.json and private user-directory permissions",
            );
        }
        writeln!(
            io::stderr().lock(),
            "jecode: invalid account options; use --help"
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
        Some(arg) if arg == OsStr::new("--sessions") => {
            match jecode::session::persistence::list() {
                Ok(sessions) => {
                    let mut out = io::stdout().lock();
                    if sessions.is_empty() {
                        writeln!(out, "No saved sessions.")?;
                    }
                    for session in sessions {
                        if session.model.is_none() {
                            writeln!(
                                out,
                                "{}  Unreadable session / file kept on disk",
                                session.id
                            )?;
                            continue;
                        }
                        let title: String = session
                            .title
                            .chars()
                            .map(|c| if c.is_control() { ' ' } else { c })
                            .collect();
                        writeln!(
                            out,
                            "{}  {}  {} turns  {}",
                            session.id,
                            session.model.map_or("unavailable", |model| model.id()),
                            session.turns,
                            title
                        )?;
                    }
                    Ok(0)
                }
                Err(_) => {
                    writeln!(
                        io::stderr().lock(),
                        "jecode: cannot list saved sessions; check ~/.jecode/v1/sessions"
                    )?;
                    Ok(2)
                }
            }
        }
        Some(arg) if arg == OsStr::new("--logout") => {
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
        None => {
            writeln!(
                io::stderr().lock(),
                "jecode: use --account for a conversation or --demo for an offline preview; see --help"
            )?;
            Ok(2)
        }
        Some(_) => {
            // Do not echo arbitrary arguments, which may contain secrets or terminal escapes.
            writeln!(io::stderr().lock(), "jecode: unknown option; use --help")?;
            Ok(2)
        }
    }
}

fn account_options(
    mut args: impl Iterator<Item = std::ffi::OsString>,
) -> Option<(Option<jecode::session::Model>, Option<std::ffi::OsString>)> {
    let mut model = None;
    let mut workspace = None;
    while let Some(option) = args.next() {
        if option == "--model" && model.is_none() {
            model = Some(match args.next()?.to_str()? {
                "gpt-5.6-luna" => jecode::session::Model::Luna,
                "gpt-5.6-terra" => jecode::session::Model::Terra,
                _ => return None,
            });
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
    Some((model, workspace))
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
