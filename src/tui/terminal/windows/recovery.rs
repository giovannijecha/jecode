use super::*;

impl Terminal {
    pub(super) fn restore_modes(&self) {
        let Some((input, output, code_page)) = self.modes else {
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
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null());
        let recovery = crate::auth::isolate_environment(&mut command).spawn();
        let Ok(mut child) = recovery else {
            eprintln!("Could not restore Windows console modes.");
            return;
        };
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return,
                Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                    thread::sleep(Duration::from_millis(10))
                }
                _ => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        let _ = write!(io::stdout(), "{}", super::super::LEAVE);
        let _ = io::stdout().flush();
        eprintln!("Could not restore Windows console modes.");
    }
}
