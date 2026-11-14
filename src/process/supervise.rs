use super::*;

impl Started {
    #[cfg(windows)]
    pub(crate) fn bridge_pid(&self) -> u32 {
        self.target_pid
    }

    /// The root's exit while supervised descendants keep the boundary open.
    pub fn watch(&self) -> Watch {
        self.watch.clone()
    }

    /// Waits for completion, for `timeout` measured from launch, or for
    /// `cancellation`; the latter two terminate the supervised boundary.
    pub fn supervise(
        self,
        timeout: Option<Duration>,
        cancellation: &Cancellation,
        observer: &mut impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<Output, RunError> {
        let Self {
            mut child,
            stdout,
            stderr,
            mut events,
            writer,
            #[cfg(windows)]
            mut completion,
            #[cfg(windows)]
            control,
            #[cfg(windows)]
                target_pid: _,
            #[cfg(unix)]
                owner: _owner,
            #[cfg(unix)]
            pty_cleanup_receipt,
            started,
            idle,
            watch: _watch,
        } = self;
        let mut last_activity = started;
        let mut status = None;
        let mut timed_out = false;
        let mut cancelled = false;
        let mut observer_error = None;
        let mut cleanup_error = None;
        #[cfg(windows)]
        let mut terminated = false;
        // Stops the boundary; Ok(true) when the job's emptiness was confirmed.
        macro_rules! terminate_owned {
            ($child:expr) => {{
                #[cfg(windows)]
                {
                    windows::terminate($child, control.as_ref(), &mut completion)
                }
                #[cfg(unix)]
                {
                    unix::terminate($child).map(|()| false)
                }
            }};
        }

        loop {
            events.collect();
            for chunk in events.chunks() {
                last_activity = Instant::now();
                if observer_error.is_none() {
                    observer_error = observer(&chunk).err();
                }
            }
            if observer_error.is_some()
                || stdout.lock().unwrap().error.is_some()
                || stderr.lock().unwrap().error.is_some()
            {
                #[cfg(windows)]
                {
                    terminated = true;
                }
                cleanup_error = terminate_owned!(&mut child).err();
                break;
            }
            if status.is_none() {
                match child.try_wait() {
                    Ok(value) => status = value,
                    Err(error) => {
                        let cleanup = terminate_owned!(&mut child).err();
                        return Err(format!(
                            "Could not wait for process: {error}{}",
                            cleanup.map_or(String::new(), |error| format!("; {error}"))
                        )
                        .into());
                    }
                }
            }
            if status.is_some()
                && events.readers_finished()
                && writer.as_ref().is_none_or(JoinHandle::is_finished)
            {
                break;
            }
            // On Unix the child is the root itself. Its exit is news only while
            // some process still holds the command's output open.
            #[cfg(unix)]
            if let Some(exit) = status
                && _watch.root().is_none()
                && !events.readers_finished()
            {
                _watch.record(RootExit {
                    pid: child.id(),
                    exit_code: exit.code(),
                    elapsed: started.elapsed(),
                    remaining: None,
                });
            }
            if cancellation.requested()
                || timeout.is_some_and(|timeout| started.elapsed() >= timeout)
                || idle.is_some_and(|idle| last_activity.elapsed() >= idle)
            {
                cancelled = cancellation.requested();
                timed_out = !cancelled;
                #[cfg(windows)]
                {
                    terminated = true;
                }
                let confirmed = match terminate_owned!(&mut child) {
                    Ok(confirmed) => confirmed,
                    Err(error) => {
                        cleanup_error = Some(error);
                        false
                    }
                };
                #[cfg(unix)]
                {
                    status = child.try_wait().ok().flatten().or(status);
                }
                // An inherited pipe must not make an already timed-out command block forever.
                #[cfg(unix)]
                let pty_owned = pty_cleanup_receipt.is_some();
                #[cfg(windows)]
                let pty_owned = false;
                let drain_limit = if confirmed || pty_owned {
                    Duration::from_secs(5)
                } else {
                    Duration::from_millis(250)
                };
                let deadline = Instant::now() + drain_limit;
                while !events.readers_finished() {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    events.wait(remaining);
                }
                if !events.readers_finished() {
                    cleanup_error.get_or_insert_with(|| {
                        "Process cleanup could not be confirmed: output capture remains active"
                            .into()
                    });
                }
                #[cfg(unix)]
                if let Some(mut receipt) = pty_cleanup_receipt
                    && let Err(error) = unix::cleanup_receipt(&mut receipt, deadline)
                {
                    cleanup_error.get_or_insert(error);
                }
                break;
            }
            // Output and stream ends wake the loop at once; the interval
            // bounds how late exit, cancellation and deadlines are noticed.
            events.wait(Duration::from_millis(10));
        }
        #[cfg(windows)]
        if terminated {
            windows::reap(&mut child);
        }

        events.collect();
        for chunk in events.chunks() {
            if observer_error.is_none() {
                observer_error = observer(&chunk).err();
            }
        }
        let cleanup = |error: Option<String>| error.map_or(String::new(), |e| format!("; {e}"));
        if let Some(error) = observer_error {
            return Err(format!("{error}{}", cleanup(cleanup_error)).into());
        }
        let mut stdout = stdout.lock().unwrap();
        let mut stderr = stderr.lock().unwrap();
        // A failed capture is why the run stopped; cleanup only adds detail.
        if let Some(error) = stdout.error.as_ref().or(stderr.error.as_ref()) {
            return Err(format!(
                "Could not read process output: {error}{}",
                cleanup(cleanup_error)
            )
            .into());
        }
        if let Some(error) = cleanup_error {
            let message = if cancelled {
                format!("Operation cancelled; {error}")
            } else if timed_out {
                format!("Process timed out; {error}")
            } else {
                error
            };
            return Err(message.into());
        }

        // A stopped run consumed the report while terminating; the
        // supervisor's exit code is not the command's then.
        #[cfg(windows)]
        let native_exit_code = match completion {
            _ if terminated => None,
            Some(report) => Some(windows::completion(report).map_err(RunError::from)?),
            None => status.and_then(|status| status.code()),
        };

        let input_error = if !timed_out && !cancelled {
            writer
                .filter(JoinHandle::is_finished)
                .and_then(|writer| match writer.join() {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(format!("Could not send process input: {error}")),
                    Err(_) => Some("Process input worker failed".into()),
                })
        } else {
            None
        };
        Ok(Output {
            stdout: std::mem::take(&mut stdout.bytes),
            stderr: std::mem::take(&mut stderr.bytes),
            input_error,
            exit_code: {
                #[cfg(windows)]
                {
                    native_exit_code
                }
                #[cfg(unix)]
                {
                    status.and_then(|status| status.code())
                }
            },
            timed_out,
            stdout_bytes: stdout.total_bytes,
            stderr_bytes: stderr.total_bytes,
            stdout_truncated: stdout.truncated,
            cancelled,
        })
    }
}
