//! Standalone device sign-in. Native terminal polling makes Esc/Ctrl+C cancellable.
use super::{Key, platform};
use crate::{
    providers::openai_account::{
        auth,
        client::{Client, Error},
    },
    tls::Budget,
};
use std::{
    io::{self, IsTerminal, Write},
    ops::ControlFlow,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub(super) fn run() -> io::Result<bool> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("login needs an interactive terminal"));
    }
    let mut terminal = platform::Terminal::open()?;
    drive(
        || terminal.poll(),
        io::stdout().lock(),
        |budget, code| Client::connect(budget, code).map(|_| ()),
    )
}

fn drive(
    mut poll: impl FnMut() -> io::Result<Vec<Key>>,
    mut output: impl Write,
    connect: impl for<'a> FnOnce(
        &Budget<'a>,
        &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), Error>
    + Send
    + 'static,
) -> io::Result<bool> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let (codes, received) = mpsc::channel::<String>();
    let worker = thread::Builder::new()
        .name("jecode-login".into())
        .spawn(move || {
            let mut shown = false;
            let result = connect(
                &Budget {
                    cancelled: &worker_cancelled,
                    deadline: Some(Instant::now() + Duration::from_secs(900)),
                },
                &mut |code| {
                    shown = true;
                    if codes.send(code.to_owned()).is_ok() {
                        ControlFlow::Continue(())
                    } else {
                        ControlFlow::Break(())
                    }
                },
            );
            (result, shown)
        })?;
    let mut input_error = writeln!(
        output,
        "Checking saved Jecode account / Esc or Ctrl+C cancels\r"
    )
    .err();
    while !worker.is_finished() {
        while let Ok(code) = received.try_recv() {
            if input_error.is_none() {
                input_error = writeln!(
                    output,
                    "Sign in at {}\r\nEnter code: {code}\r\nWaiting for approval / Esc cancels\r",
                    auth::VERIFICATION_URL
                )
                .and_then(|()| output.flush())
                .err();
            }
        }
        if input_error.is_some() {
            cancelled.store(true, Ordering::Release);
            break;
        }
        match poll() {
            Ok(keys)
                if keys
                    .iter()
                    .any(|key| matches!(key, Key::Escape | Key::Interrupt | Key::Quit)) =>
            {
                cancelled.store(true, Ordering::Release);
            }
            Ok(_) => {}
            Err(error) => {
                cancelled.store(true, Ordering::Release);
                input_error = Some(error);
                break;
            }
        }
    }
    let (result, shown) = worker
        .join()
        .map_err(|_| io::Error::other("login worker stopped"))?;
    if let Some(error) = input_error {
        return Err(error);
    }
    while let Ok(code) = received.try_recv() {
        writeln!(
            output,
            "Sign in at {}\r\nEnter code: {code}\r\nWaiting for approval / Esc cancels\r",
            auth::VERIFICATION_URL
        )?;
    }
    if cancelled.load(Ordering::Acquire) && result.is_err() {
        writeln!(output, "Sign-in cancelled. Run jecode login to retry.\r")?;
        return Ok(false);
    }
    match result {
        Ok(()) => {
            writeln!(
                output,
                "{}\r",
                if shown {
                    "Signed in to Jecode."
                } else {
                    "Already signed in to Jecode."
                }
            )?;
            Ok(true)
        }
        Err(error) => Err(io::Error::other(format!(
            "{error}; run jecode login to retry"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type SyntheticConnect =
        fn(&Budget<'_>, &mut dyn FnMut(&str) -> ControlFlow<()>) -> Result<(), Error>;
    #[test]
    fn standalone_login_reports_success_reuse_failure_and_cancellation_with_synthetic_backends() {
        let run = |connect: SyntheticConnect, cancel: bool| {
            let mut output = Vec::new();
            let result = drive(
                || {
                    thread::sleep(Duration::from_millis(1));
                    Ok(if cancel { vec![Key::Escape] } else { vec![] })
                },
                &mut output,
                connect,
            );
            (result, String::from_utf8(output).unwrap())
        };
        let (result, output) = run(
            |_, code| {
                let _ = code("SYNTHETIC-CODE");
                Ok(())
            },
            false,
        );
        assert!(result.unwrap());
        assert!(output.contains("SYNTHETIC-CODE") && output.contains("Signed in"));
        let (result, output) = run(|_, _| Ok(()), false);
        assert!(result.unwrap());
        assert!(output.contains("Already signed in"));
        let (result, _) = run(|_, _| Err(Error::Expired), false);
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("run jecode login to retry")
        );
        let (result, output) = run(
            |budget, code| {
                let _ = code("SYNTHETIC-CODE");
                loop {
                    budget.check()?;
                    thread::sleep(Duration::from_millis(1));
                }
            },
            true,
        );
        assert!(!result.unwrap());
        assert!(output.contains("cancelled"));
    }
}
