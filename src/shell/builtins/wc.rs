use super::*;
use std::io::Read;

enum Count {
    Lines,
    Bytes,
}

/// Counts newline bytes or input bytes without buffering entire files.
pub(super) fn wc(shell: &Shell, args: &[String], io: &Io) -> Outcome {
    let mut kind = None;
    let mut files = Vec::new();
    let mut positional = false;
    for arg in &args[1..] {
        match arg.as_str() {
            "--" if !positional => positional = true,
            "-l" | "-c" if !positional => {
                if kind.is_some() {
                    return Err("wc: use only one of -l or -c".into());
                }
                kind = Some(if arg == "-l" {
                    Count::Lines
                } else {
                    Count::Bytes
                });
            }
            value if !positional && value.starts_with('-') && value != "-" => {
                return Err(format!("wc: unsupported option: {value}"));
            }
            _ => files.push(arg.as_str()),
        }
    }
    let kind = kind.ok_or("wc: expected -l or -c")?;
    let named = !files.is_empty();
    if !named {
        files.push("-");
    }

    let mut output = String::new();
    let mut errors = String::new();
    let mut total = 0u64;
    let mut stdin = &io.stdin;
    for file in &files {
        let result = if *file == "-" {
            count(&mut stdin, &kind)
        } else {
            fs::File::open(shell.path(file)).and_then(|mut input| count(&mut input, &kind))
        };
        match result {
            Ok(value) => {
                total += value;
                if !named {
                    output.push_str(&format!("{value}\n"));
                } else {
                    output.push_str(&format!("{value} {file}\n"));
                }
            }
            Err(error) => errors.push_str(&format!("wc: {file}: {error}\n")),
        }
    }
    if files.len() > 1 {
        output.push_str(&format!("{total} total\n"));
    }
    Ok((
        i32::from(!errors.is_empty()),
        output.into_bytes(),
        errors.into_bytes(),
    ))
}

fn count(reader: &mut impl Read, kind: &Count) -> io::Result<u64> {
    match kind {
        Count::Bytes => io::copy(reader, &mut io::sink()),
        Count::Lines => {
            let mut lines = 0;
            let mut buffer = [0u8; 8192];
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                lines += buffer[..read].iter().filter(|&&byte| byte == b'\n').count() as u64;
            }
            Ok(lines)
        }
    }
}
