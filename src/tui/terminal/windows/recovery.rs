use super::*;

impl Terminal {
    pub(super) fn restore_modes(&self) {
        let Some((input, output, code_page)) = self.modes else {
            return;
        };
        let script = format!(
            "Add-Type -TypeDefinition @'\n{}\n'@\n[JecodeConsole]::Restore({}, {}, {})",
            include_str!("../../terminal.cs"),
            input,
            output,
            code_page
        );
        let recovery = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null())
            .env_remove("OPENROUTER_API_KEY")
            .spawn();
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
