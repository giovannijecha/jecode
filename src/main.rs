use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

const HELP: &str = "Jecode — owned coding harness\n\nUsage: jecode [--workspace PATH] [--model MODEL] [--effort LEVEL] [--access local|workspace]\n       jecode [--workspace PATH] resume [SESSION_ID]\n       jecode [--workspace PATH] sessions\n       jecode chat | login | logout\n\n  jecode        Start in the current directory with your saved account\n  login         Sign in without creating a conversation; Esc or Ctrl+C cancels\n  logout        Remove Jecode's locally saved account access\n  resume        Choose a saved conversation in this directory, or reopen SESSION_ID\n  sessions      List conversations in this directory\n  chat          Start a conversation without file tools, associated with this directory\n\n  --workspace PATH  Select another directory (also for chat, sessions and resume)\n  --model MODEL     Account model identifier for this new conversation\n  --effort LEVEL    Reasoning effort, or default to omit the provider field\n  --access PROFILE  local (default) or workspace, for file-tool sessions\n  --demo            Offline terminal preview\n  -h, --help        Show this help\n  -V, --version     Show the native version\n\nInside Jecode, type / for local commands including /login and /logout.\nFile changes and commands require approval. Commands run with your user permissions.\nCredentials, settings and sessions use ordinary JSON in ~/.jecode/v1/.\nResume never changes directories or replays historical tools.\nLegacy --account, --resume, --sessions and --logout remain supported.\n";

enum Operation {
    Start,
    Chat,
    LegacyAccount,
    Resume(Option<String>),
    Sessions,
    Login,
    Logout,
    Demo,
    Help,
    Version,
}
struct Options {
    operation: Operation,
    workspace: Option<OsString>,
    model: Option<jecode::session::Model>,
    effort: Option<Option<String>>,
    access: Option<jecode::workspace::Access>,
}
fn parse(mut args: impl Iterator<Item = OsString>) -> Option<Options> {
    let mut operation = None;
    let mut workspace = None;
    let mut model = None;
    let mut effort = None;
    let mut access = None;
    while let Some(argument) = args.next() {
        match argument.to_str()? {
            "--workspace" if workspace.is_none() => {
                let value = args.next()?;
                if value.is_empty() {
                    return None;
                }
                workspace = Some(value);
            }
            "--model" if model.is_none() => {
                model = Some(jecode::session::Model::new(args.next()?.to_str()?, None)?);
            }
            "--effort" if effort.is_none() => {
                let value = args.next()?;
                let value = value.to_str()?;
                effort = Some(if value == "default" {
                    None
                } else {
                    jecode::session::Model::new("probe", Some(value))?;
                    Some(value.into())
                });
            }
            "--access" if access.is_none() => {
                access = Some(jecode::workspace::Access::parse(args.next()?.to_str()?)?);
            }
            "resume" | "--resume" if operation.is_none() => {
                operation = Some(Operation::Resume(None))
            }
            "sessions" | "--sessions" if operation.is_none() => {
                operation = Some(Operation::Sessions)
            }
            "login" if operation.is_none() => operation = Some(Operation::Login),
            "logout" | "--logout" if operation.is_none() => operation = Some(Operation::Logout),
            "chat" if operation.is_none() => operation = Some(Operation::Chat),
            "--account" if operation.is_none() => operation = Some(Operation::LegacyAccount),
            "--demo" if operation.is_none() => operation = Some(Operation::Demo),
            "--help" | "-h" if operation.is_none() => operation = Some(Operation::Help),
            "--version" | "-V" if operation.is_none() => operation = Some(Operation::Version),
            value if !value.starts_with('-') => match &mut operation {
                Some(Operation::Resume(id @ None)) => *id = Some(value.into()),
                _ => return None,
            },
            _ => return None,
        }
    }
    let operation = operation.unwrap_or(Operation::Start);
    if (model.is_some() || effort.is_some() || access.is_some())
        && !matches!(
            operation,
            Operation::Start | Operation::Chat | Operation::LegacyAccount | Operation::Resume(_)
        )
    {
        return None;
    }
    if workspace.is_some()
        && matches!(
            operation,
            Operation::Login
                | Operation::Logout
                | Operation::Demo
                | Operation::Help
                | Operation::Version
        )
    {
        return None;
    }
    if access.is_some() && matches!(operation, Operation::Chat) {
        return None;
    }
    if access.is_some() && matches!(operation, Operation::Resume(_)) {
        return None;
    }
    if access.is_some() && matches!(operation, Operation::LegacyAccount) && workspace.is_none() {
        return None;
    }
    Some(Options {
        operation,
        workspace,
        model,
        effort,
        access,
    })
}

fn selected_directory(workspace: Option<&OsStr>) -> io::Result<jecode::session::scope::Directory> {
    jecode::session::scope::Directory::open(Path::new(workspace.unwrap_or(OsStr::new("."))))
}
fn run(args: impl Iterator<Item = OsString>) -> io::Result<u8> {
    let Some(options) = parse(args) else {
        writeln!(
            io::stderr().lock(),
            "jecode: invalid options; use jecode --help"
        )?;
        return Ok(2);
    };
    let Options {
        operation,
        workspace,
        model,
        effort,
        access,
    } = options;
    match operation {
        Operation::Start | Operation::Chat | Operation::LegacyAccount => {
            let directory = match selected_directory(workspace.as_deref()) {
                Ok(directory) => directory,
                Err(_) => {
                    return diagnostic(
                        "cannot open the selected working directory; use an available local directory",
                    );
                }
            };
            let file_tools = matches!(operation, Operation::Start)
                || matches!(operation, Operation::LegacyAccount) && workspace.is_some();
            let tools = if file_tools {
                match jecode::workspace::Workspace::open(directory.path()) {
                    Ok(workspace) => Some(workspace),
                    Err(_) => {
                        return diagnostic(
                            "cannot open the selected workspace; use a supported local directory",
                        );
                    }
                }
            } else {
                None
            };
            terminal_result(
                jecode::terminal::configured_account(model, effort, directory, tools, access),
                "cannot start a session; check ~/.jecode/v1/settings.json and private user-directory permissions",
            )
        }
        Operation::Sessions | Operation::Resume(_) => {
            if matches!(operation, Operation::Resume(_)) && (model.is_some() || effort.is_some()) {
                return diagnostic(
                    "resume uses its saved model and effort; omit --model and --effort",
                );
            }
            let directory = match selected_directory(workspace.as_deref()) {
                Ok(directory) => directory,
                Err(_) => {
                    return diagnostic(
                        "cannot open the selected working directory; use an available local directory",
                    );
                }
            };
            match operation {
                Operation::Sessions => match jecode::terminal::sessions(false, &directory) {
                    Ok(_) => Ok(0),
                    Err(_) => diagnostic("cannot list saved sessions; check ~/.jecode/v1/sessions"),
                },
                Operation::Resume(id) => {
                    let id = match id {
                        Some(id) => id,
                        None => match jecode::terminal::sessions(true, &directory) {
                            Ok(Some(id)) => id,
                            Ok(None) => return Ok(0),
                            Err(_) => {
                                return diagnostic(
                                    "cannot list saved sessions; use a supported interactive terminal and check ~/.jecode/v1/sessions",
                                );
                            }
                        },
                    };
                    let result = jecode::terminal::resume(&id, directory);
                    match result {
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::PermissionDenied
                                    | io::ErrorKind::InvalidData
                                    | io::ErrorKind::NotFound
                            ) =>
                        {
                            writeln!(io::stderr().lock(), "jecode: {error}")?;
                            Ok(2)
                        }
                        result => terminal_result(
                            result,
                            "cannot resume; use jecode sessions, close any other owner, and check the selected directory and ~/.jecode/v1/settings.json",
                        ),
                    }
                }
                _ => unreachable!(),
            }
        }
        Operation::Login => match jecode::terminal::login() {
            Ok(true) => Ok(0),
            Ok(false) => Ok(130),
            Err(error) => {
                writeln!(io::stderr().lock(), "jecode: {error}")?;
                Ok(2)
            }
        },
        Operation::Logout => {
            let cancelled = std::sync::atomic::AtomicBool::new(false);
            let budget = jecode::tls::Budget {
                cancelled: &cancelled,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(5),
            };
            match jecode::providers::openai_account::client::Client::logout(&budget) {
                Ok(()) => {
                    writeln!(io::stdout().lock(), "Jecode is signed out locally.")?;
                    Ok(0)
                }
                Err(error) => {
                    writeln!(io::stderr().lock(), "jecode: {error}")?;
                    Ok(2)
                }
            }
        }
        Operation::Demo => terminal_result(
            jecode::terminal::demo(),
            "terminal initialization or I/O failed",
        ),
        Operation::Help => {
            io::stdout().lock().write_all(HELP.as_bytes())?;
            Ok(0)
        }
        Operation::Version => {
            writeln!(io::stdout().lock(), "jecode {}", jecode::VERSION)?;
            Ok(0)
        }
    }
}
fn diagnostic(message: &str) -> io::Result<u8> {
    writeln!(io::stderr().lock(), "jecode: {message}")?;
    Ok(2)
}
fn terminal_result(result: io::Result<()>, context: &str) -> io::Result<u8> {
    match result {
        Ok(()) => Ok(0),
        Err(_) => {
            use io::IsTerminal;
            diagnostic(
                if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                    "requires a supported interactive terminal"
                } else {
                    context
                },
            )
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
    fn options(args: &[&str]) -> Option<Options> {
        parse(args.iter().map(OsString::from))
    }
    #[test]
    fn workspace_option_is_shared_by_start_chat_listing_and_resume() {
        assert!(matches!(options(&[]).unwrap().operation, Operation::Start));
        assert!(matches!(
            options(&["--workspace", "folder", "resume", "id"])
                .unwrap()
                .operation,
            Operation::Resume(Some(_))
        ));
        assert!(matches!(
            options(&["sessions", "--workspace", "folder"])
                .unwrap()
                .operation,
            Operation::Sessions
        ));
        assert!(matches!(
            options(&["chat", "--workspace", "folder"])
                .unwrap()
                .operation,
            Operation::Chat
        ));
        assert!(options(&["login", "--workspace", "folder"]).is_none());
        assert!(options(&["resume", "one", "two"]).is_none());
    }
    #[test]
    fn model_and_effort_options_cover_chat_and_reject_resume_overrides() {
        let selected = options(&["chat", "--model", "future-model", "--effort", "xhigh"]).unwrap();
        assert!(matches!(selected.operation, Operation::Chat));
        assert_eq!(selected.model.unwrap().id(), "future-model");
        assert_eq!(selected.effort, Some(Some("xhigh".into())));
        assert_eq!(
            options(&["--effort", "default"]).unwrap().effort,
            Some(None)
        );
        assert!(options(&["--model", "bad\nmodel"]).is_none());
        assert!(options(&["--effort", ""]).is_none());
        // Parsing retains the override long enough for `run` to issue a
        // specific resume diagnostic instead of silently discarding it.
        let resume = options(&["resume", "s-test", "--model", "future-model"]).unwrap();
        assert!(matches!(resume.operation, Operation::Resume(_)));
    }
}
