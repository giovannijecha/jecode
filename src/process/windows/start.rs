//! Supervised command startup and persistent-input handoff.
use super::*;

pub(super) fn spawn(
    command: &Command,
    input: Option<Vec<u8>>,
    cancellation: &Cancellation,
    persistent: bool,
) -> Result<Spawned, RunError> {
    // The supervisor serializes argv instead of calling Command::spawn on
    // the target. Preserve the native API's rejection of embedded NULs.
    let contains_nul = |value: &OsStr| value.encode_wide().any(|unit| unit == 0);
    if contains_nul(command.get_program()) || command.get_args().any(contains_nul) {
        return Err(RunError {
            started: false,
            message: "Executable and arguments must not contain NUL characters".into(),
        });
    }
    if cancellation.requested() {
        return Err(RunError {
            started: false,
            message: "Operation cancelled".into(),
        });
    }
    let executable = crate::native::helper("supervisor").map_err(|message| RunError {
        started: false,
        message,
    })?;
    let mut helper = Command::new(executable);
    configure(&mut helper, command);
    let mut child = helper.spawn().map_err(|error| RunError {
        started: false,
        message: if error.kind() == io::ErrorKind::Interrupted {
            "Operation cancelled".into()
        } else {
            format!("Could not start Windows process supervisor: {error}")
        },
    })?;
    let pipe_name = format!(
        "jecode-process-{}-{}",
        std::process::id(),
        NEXT_PIPE.fetch_add(1, Ordering::Relaxed)
    );
    let config = encode_command(command, &pipe_name);
    let mut stdin = child.stdin.take().unwrap();
    let (input_sender, input_receiver) = mpsc::sync_channel(1);
    let writer = thread::spawn(move || {
        stdin.write_all(&config)?;
        if persistent {
            input_sender
                .send(stdin)
                .map_err(|_| io::ErrorKind::BrokenPipe)?;
        } else if let Some(input) = input {
            stdin.write_all(&input)?;
        }
        Ok(())
    });
    let mut stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let handshake = thread::spawn(move || {
        let result = read_handshake(&mut stdout, |stage| {
            let _ = sender.send(Handshake::Stage(stage));
        });
        let _ = sender.send(Handshake::Done(stdout, result));
    });
    let started = Instant::now();
    let mut last_stage = 0;
    loop {
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(Handshake::Stage(stage)) => last_stage = stage,
            Ok(Handshake::Done(stdout, Ok(target_pid))) => {
                let _ = handshake.join();
                // The supervisor creates both channels before its handshake.
                let mut completion_file =
                    connect(&mut child, &pipe_name, "completion", false, cancellation)?;
                let control = connect(
                    &mut child,
                    &format!("{pipe_name}-stop"),
                    "stop",
                    true,
                    cancellation,
                )?;
                let (report, completion) = mpsc::channel();
                thread::spawn(move || {
                    let _ = report.send(read_completion(&mut completion_file));
                });
                let retained_input = if persistent {
                    match input_receiver.recv_timeout(Duration::from_secs(1)) {
                        Ok(stdin) => Some(stdin),
                        Err(_) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            let _ = writer.join();
                            return Err(RunError::from(
                                "Windows process input channel did not initialize".to_string(),
                            ));
                        }
                    }
                } else {
                    None
                };
                return Ok(Spawned {
                    child,
                    stdout,
                    writer,
                    completion: Some(completion),
                    control: Some(control),
                    target_pid,
                    stdin: retained_input,
                });
            }
            Ok(Handshake::Done(_, Err(message))) => {
                let _ = handshake.join();
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                return Err(RunError {
                    started: false,
                    message,
                });
            }
            Err(mpsc::RecvTimeoutError::Timeout)
                if started.elapsed() < Duration::from_secs(15)
                    && !cancellation.requested()
                    && matches!(child.try_wait(), Ok(None)) => {}
            Err(_) => {
                let exited = child.try_wait().ok().flatten();
                let _ = child.kill();
                let _ = child.wait();
                let _ = handshake.join();
                let _ = writer.join();
                // A supervisor that exited on its own may have reported why
                // after the last receive; prefer that over a generic message.
                let mut reported = None;
                for event in receiver.try_iter() {
                    match event {
                        Handshake::Stage(stage) => last_stage = stage,
                        Handshake::Done(_, Err(message)) if message != HANDSHAKE_STOPPED => {
                            reported = Some(message);
                        }
                        Handshake::Done(..) => {}
                    }
                }
                return Err(RunError {
                    started: false,
                    message: if cancellation.requested() {
                        "Operation cancelled".into()
                    } else if let Some(message) = reported {
                        message
                    } else {
                        format!(
                            "Windows process supervisor did not initialize (last stage: {}; exit code: {})",
                            stage_name(last_stage),
                            exited.and_then(|status| status.code()).map_or_else(
                                || "still running".to_string(),
                                |code| code.to_string()
                            )
                        )
                    },
                });
            }
        }
    }
}
