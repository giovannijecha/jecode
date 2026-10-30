use super::*;
use std::io::Read;

/// `tr -d`, streaming standard input to standard output.
pub(super) fn tr(shell: &mut Shell, args: &[String], io: &Io) -> Outcome {
    if args.len() != 3 || args[1] != "-d" {
        return Err("tr: only -d CHARACTERS is supported".into());
    }
    let pattern = args[2]
        .replace("\\n", "\n")
        .replace("\\r", "\r")
        .replace("\\t", "\t");
    let delete = pattern.as_bytes();
    let mut reader = &io.stdin;
    let mut writer = &io.stdout;
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("tr: {error}"))?;
        if count == 0 {
            break;
        }
        let filtered: Vec<u8> = buffer[..count]
            .iter()
            .copied()
            .filter(|byte| !delete.contains(byte))
            .collect();
        match writer.write_all(&filtered) {
            Ok(()) => {}
            Err(error) if closed(&error) => {
                shell.broken_output = true;
                return Ok((1, Vec::new(), Vec::new()));
            }
            Err(error) => return Err(format!("tr: {error}")),
        }
    }
    if let Err(error) = writer.flush() {
        if closed(&error) {
            shell.broken_output = true;
            return Ok((1, Vec::new(), Vec::new()));
        }
        return Err(format!("tr: {error}"));
    }
    ok(Vec::new())
}
