use super::*;

impl Terminal {
    pub(super) fn restore_modes(&self) {
        let Some((input, output, code_page, title)) = &self.modes else {
            return;
        };
        let Ok(executable) = crate::native::helper("terminal") else {
            return;
        };
        let mut command = Command::new(executable);
        command
            .args([
                "restore".into(),
                input.to_string(),
                output.to_string(),
                code_page.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null());
        let recovery = crate::auth::isolate_environment(&mut command).spawn();
        let Ok(mut child) = recovery else {
            eprintln!("Could not restore Windows console modes.");
            return;
        };
        // A full console title can exceed the command-line limit once encoded.
        // Feed it on a pipe while retaining the existing recovery deadline.
        let mut input = child.stdin.take().unwrap();
        let title = title.clone();
        let writer = thread::spawn(move || writeln!(input, "{title}"));
        let started = Instant::now();
        let restored = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                    thread::sleep(Duration::from_millis(10))
                }
                _ => break false,
            }
        };
        if !restored {
            let _ = child.kill();
        }
        let _ = child.wait();
        let sent = writer.join().is_ok_and(|result| result.is_ok());
        if restored && sent {
            return;
        }
        let _ = write!(io::stdout(), "{}", super::super::LEAVE);
        let _ = io::stdout().flush();
        eprintln!("Could not restore Windows console modes.");
    }
}
