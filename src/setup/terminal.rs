use super::{Prompter, SignIn};
use crate::cancel::Cancellation;
use crate::input::read_line;
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct Terminal<'a, R, W> {
    input: &'a mut R,
    output: &'a mut W,
    bash: &'a Path,
}

impl<'a, R: BufRead, W: Write> Terminal<'a, R, W> {
    pub fn new(input: &'a mut R, output: &'a mut W, bash: &'a Path) -> Self {
        Self {
            input,
            output,
            bash,
        }
    }
}

impl<R: BufRead, W: Write> Prompter for Terminal<'_, R, W> {
    fn message(&mut self, text: &str) -> Result<(), String> {
        writeln!(self.output, "{text}")
            .and_then(|_| self.output.flush())
            .map_err(|error| error.to_string())
    }

    fn input(&mut self, prompt: &str) -> Result<Option<String>, String> {
        write!(self.output, "{prompt}")
            .and_then(|_| self.output.flush())
            .map_err(|error| error.to_string())?;
        read_line(self.input)
    }

    fn secret(&mut self, prompt: &str) -> Result<Option<String>, String> {
        write!(self.output, "{prompt}")
            .and_then(|_| self.output.flush())
            .map_err(|error| error.to_string())?;
        // The owned terminal adapter disables echo and returns the key through
        // a private pipe; no credential appears in a process argument.
        let helper = crate::native::helper(if cfg!(windows) {
            "terminal"
        } else {
            "terminal_unix"
        })?;
        let mut command = Command::new(helper);
        command
            .arg("secret")
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let output = crate::auth::isolate_environment(&mut command)
            .output()
            .map_err(|error| format!("Could not read the key from the terminal: {error}"))?;
        match output.status.code() {
            Some(0) => String::from_utf8(output.stdout)
                .map(Some)
                .map_err(|_| "The key must be valid UTF-8".into()),
            Some(2) | Some(130) => Ok(None),
            Some(3) => Err("The API key exceeds the 512-character limit".into()),
            _ => Err("Could not read the key from the terminal".into()),
        }
    }

    fn with_sign_in_cancel<T>(
        &mut self,
        operation: impl FnOnce(&mut Self, Cancellation) -> Result<T, String>,
    ) -> Result<SignIn<T>, String> {
        self.message("Press Enter during ChatGPT sign-in to return to provider selection.")?;
        let cancellation = Cancellation::default();
        let monitor = SignInMonitor::start(self.bash, cancellation.clone())?;
        let result = operation(self, cancellation.clone());
        monitor.stop()?;
        match result {
            Ok(value) => Ok(SignIn::Completed(value)),
            Err(error) if cancellation.requested() && error == "Operation cancelled" => {
                Ok(SignIn::Back)
            }
            Err(error) => Err(error),
        }
    }
}

struct SignInMonitor {
    stop: Sender<()>,
    watcher: Option<JoinHandle<Result<(), String>>>,
}

impl SignInMonitor {
    fn start(bash: &Path, cancellation: Cancellation) -> Result<Self, String> {
        // A separate process owns the pending console read. It can be killed and
        // reaped before the next prompt, unlike a blocked Rust stdin thread.
        let _ = bash;
        let helper = crate::native::helper(if cfg!(windows) {
            "terminal"
        } else {
            "terminal_unix"
        })?;
        let mut command = Command::new(helper);
        command
            .arg("wait-enter")
            .stdin(Stdio::inherit())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = crate::auth::isolate_environment(&mut command)
            .spawn()
            .map_err(|error| format!("Could not monitor ChatGPT sign-in input: {error}"))?;
        let (stop, receiver) = mpsc::channel();
        let watcher = thread::spawn(move || {
            let result = loop {
                if receiver.try_recv().is_ok() {
                    break Ok(());
                }
                match child.try_wait() {
                    Ok(Some(_)) => {
                        cancellation.cancel();
                        break Ok(());
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(20)),
                    Err(error) => {
                        cancellation.cancel();
                        break Err(format!("Could not monitor ChatGPT sign-in input: {error}"));
                    }
                }
            };
            if !matches!(child.try_wait(), Ok(Some(_)))
                && let Err(error) = child.kill()
                && !matches!(child.try_wait(), Ok(Some(_)))
            {
                return Err(format!("Could not stop ChatGPT sign-in input: {error}"));
            }
            child
                .wait()
                .map_err(|error| format!("Could not reap ChatGPT sign-in input: {error}"))?;
            result
        });
        Ok(Self {
            stop,
            watcher: Some(watcher),
        })
    }

    fn stop(mut self) -> Result<(), String> {
        self.join()
    }

    fn join(&mut self) -> Result<(), String> {
        let _ = self.stop.send(());
        if let Some(watcher) = self.watcher.take() {
            watcher
                .join()
                .map_err(|_| "ChatGPT sign-in input monitor stopped unexpectedly".to_string())??;
        }
        Ok(())
    }
}

impl Drop for SignInMonitor {
    fn drop(&mut self) {
        let _ = self.join();
    }
}

#[cfg(test)]
pub(crate) fn native_sign_in_fixture(case: &str) {
    use std::io;

    let bash = crate::tools::find_bash().unwrap();
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut terminal = Terminal::new(&mut input, &mut output, &bash);
    let result = terminal.with_sign_in_cancel(|ui, cancellation| {
        ui.message("SETUP_SIGN_IN_WAITING")?;
        match case {
            "setup-cancel" => loop {
                cancellation.wait(Duration::from_millis(50))?;
            },
            "setup-success" => {
                thread::sleep(Duration::from_millis(150));
                Ok(())
            }
            "setup-error" => {
                thread::sleep(Duration::from_millis(150));
                Err("fixture error".into())
            }
            _ => panic!("unknown setup fixture case"),
        }
    });
    match case {
        "setup-cancel" => assert!(matches!(result, Ok(SignIn::Back))),
        "setup-success" => assert!(matches!(result, Ok(SignIn::Completed(())))),
        "setup-error" => assert!(matches!(result, Err(error) if error == "fixture error")),
        _ => unreachable!(),
    }
    let next = terminal.input("SETUP_NEXT_PROMPT: ").unwrap().unwrap();
    assert_eq!(next.trim_end_matches(['\r', '\n']), "next");
    terminal.message("SETUP_NATIVE_DONE").unwrap();
}
