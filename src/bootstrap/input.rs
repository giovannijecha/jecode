//! Piped stdin for single runs. Without a prompt argument stdin is the task;
//! with one, stdin is attached to it as data. Input is never truncated or
//! dropped: an oversized or non-UTF-8 stream is an error.

use std::io::Read;

pub const LIMIT: usize = 1024 * 1024;

pub fn read(input: &mut impl Read) -> Result<String, String> {
    let mut bytes = Vec::new();
    input
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read piped input: {error}"))?;
    if bytes.len() > LIMIT {
        return Err("Piped input exceeds the 1 MiB limit; nothing was sent".into());
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    String::from_utf8(bytes.to_vec()).map_err(|_| {
        "Piped input is not valid UTF-8; nothing was sent. In Windows PowerShell 5.1, set $OutputEncoding = [Text.UTF8Encoding]::new() first".into()
    })
}

/// The message for a run. `data` is piped stdin, when stdin is not a terminal.
pub fn compose(prompt: Option<String>, data: Option<String>) -> Result<String, String> {
    match (prompt, data) {
        (Some(prompt), Some(data)) if !data.is_empty() => {
            let newline = if data.ends_with('\n') { "" } else { "\n" };
            Ok(format!("{prompt}\n\n<stdin>\n{data}{newline}</stdin>"))
        }
        (Some(prompt), _) => Ok(prompt),
        (None, Some(data)) if !data.trim().is_empty() => Ok(data),
        (None, Some(_)) => {
            Err("No task: piped stdin was empty. Supply a PROMPT or pipe a task".into())
        }
        (None, None) => Err("No task: supply a PROMPT or pipe a task on stdin".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_and_piped_data_are_both_kept() {
        assert_eq!(
            compose(Some("Review this diff".into()), Some("+fn a() {}\n".into())).unwrap(),
            "Review this diff\n\n<stdin>\n+fn a() {}\n</stdin>"
        );
        assert_eq!(
            compose(Some("Explain".into()), Some("no newline".into())).unwrap(),
            "Explain\n\n<stdin>\nno newline\n</stdin>"
        );
        assert_eq!(
            compose(Some("Explain".into()), Some(" \n".into())).unwrap(),
            "Explain\n\n<stdin>\n \n</stdin>"
        );
        assert_eq!(
            compose(Some("Only".into()), Some(String::new())).unwrap(),
            "Only"
        );
        assert_eq!(compose(Some("Only".into()), None).unwrap(), "Only");
        assert_eq!(compose(None, Some("task\n".into())).unwrap(), "task\n");
        assert!(
            compose(None, Some(" \n".into()))
                .unwrap_err()
                .contains("empty")
        );
        assert!(compose(None, None).is_err());
    }

    #[test]
    fn piped_input_rejects_oversize_and_invalid_utf8_and_drops_a_bom() {
        assert_eq!(read(&mut &b"\xEF\xBB\xBFtask"[..]).unwrap(), "task");
        assert!(read(&mut &b"\xFF\xFE"[..]).unwrap_err().contains("UTF-8"));
        let large = vec![b'a'; LIMIT + 1];
        assert!(read(&mut &large[..]).unwrap_err().contains("1 MiB"));
        assert_eq!(read(&mut &vec![b'a'; LIMIT][..]).unwrap().len(), LIMIT);
    }
}
