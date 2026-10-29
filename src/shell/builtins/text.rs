use super::*;
use std::io::Write;

pub(super) fn tr(args: &[String], input: Option<&[u8]>, stream_direct: bool) -> Outcome {
    if args.len() != 3 || args[1] != "-d" {
        return Err("tr: only -d CHARACTERS is supported".into());
    }
    let pattern = args[2]
        .replace("\\n", "\n")
        .replace("\\r", "\r")
        .replace("\\t", "\t");
    let delete = pattern.as_bytes();
    if let Some(input) = input {
        return ok(input
            .iter()
            .copied()
            .filter(|byte| !delete.contains(byte))
            .collect());
    }
    let mut reader = io::stdin();
    let mut buffer = [0u8; 8192];
    let mut collected = Vec::new();
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
        if stream_direct {
            io::stdout()
                .write_all(&filtered)
                .map_err(|error| format!("tr: {error}"))?;
        } else {
            collected.extend(filtered);
        }
    }
    if stream_direct {
        io::stdout()
            .flush()
            .map_err(|error| format!("tr: {error}"))?;
    }
    ok(collected)
}
