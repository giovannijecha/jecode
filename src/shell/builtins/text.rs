use super::super::streams::Source;
use super::*;
use std::io::{Read, Seek, SeekFrom};

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

/// `head` and `tail` over files or standard input: `-n N`, `-N`, and for
/// `tail` also `-n +N` (from line N). Lines end at `\n`; bytes pass unchanged.
pub(super) fn head_tail(shell: &mut Shell, args: &[String], io: &Io) -> Outcome {
    let name = args[0].as_str();
    let mut count = None;
    let mut operands = Vec::new();
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        if arg == "-n" {
            let value = rest.next().ok_or(format!("{name}: -n needs a number"))?;
            count = Some(value.as_str());
        } else if let Some(value) = arg.strip_prefix("-n") {
            count = Some(value);
        } else if arg.len() > 1
            && arg.starts_with('-')
            && arg[1..].bytes().all(|byte| byte.is_ascii_digit())
        {
            count = Some(&arg[1..]);
        } else if arg.starts_with('-') && arg != "-" {
            return Err(format!(
                "{name}: only -n N, -N{} and file operands are supported",
                if name == "tail" { ", -n +N" } else { "" }
            ));
        } else {
            operands.push(arg.as_str());
        }
    }
    let count = count.unwrap_or("10");
    let (from_line, digits) = match count.strip_prefix('+') {
        Some(digits) if name == "tail" => (true, digits),
        _ => (false, count),
    };
    let number: usize = digits
        .parse()
        .map_err(|_| format!("{name}: invalid line count: {count}"))?;
    if operands.is_empty() {
        operands.push("-");
    }
    let mut writer = &io.stdout;
    let mut errors = String::new();
    let mut copy = || -> io::Result<()> {
        let mut first = true;
        for operand in &operands {
            let file = match *operand {
                "-" => None,
                path => match fs::File::open(shell.path(path)) {
                    Ok(file) => Some(file),
                    Err(error) => {
                        errors.push_str(&format!("{name}: {operand}: {error}\n"));
                        continue;
                    }
                },
            };
            if operands.len() > 1 {
                let gap = if first { "" } else { "\n" };
                writeln!(writer, "{gap}==> {operand} <==")?;
            }
            first = false;
            let source: Box<dyn Read + '_> = match &file {
                Some(file) => Box::new(file),
                None => Box::new(&io.stdin),
            };
            let mut lines = io::BufReader::new(source);
            match select(&mut lines, &mut writer, name == "head", from_line, number) {
                Err(error) if !closed(&error) => {
                    errors.push_str(&format!("{name}: {operand}: {error}\n"));
                }
                result => result?,
            }
            // The next command on this standard input reads what was not selected.
            // A pipe opened as a file cannot seek and keeps nothing, as in GNU head.
            if file.is_none()
                && let Source::File(input) = &io.stdin
            {
                let unread = lines.buffer().len() as i64;
                let _ = (&*input).seek(SeekFrom::Current(-unread));
            }
        }
        writer.flush()
    };
    match copy() {
        Ok(()) => Ok((
            i32::from(!errors.is_empty()),
            Vec::new(),
            errors.into_bytes(),
        )),
        Err(error) if closed(&error) => {
            shell.broken_output = true;
            Ok((1, Vec::new(), Vec::new()))
        }
        Err(error) => Err(format!("{name}: {error}")),
    }
}

fn select(
    lines: &mut dyn io::BufRead,
    writer: &mut dyn Write,
    head: bool,
    from_line: bool,
    number: usize,
) -> io::Result<()> {
    let mut line = Vec::new();
    let mut kept = std::collections::VecDeque::new();
    let mut seen = 0usize;
    loop {
        if head && seen == number {
            return Ok(());
        }
        line.clear();
        if lines.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        seen += 1;
        if head || (from_line && seen >= number.max(1)) {
            writer.write_all(&line)?;
        } else if !from_line && number > 0 {
            if kept.len() == number {
                kept.pop_front();
            }
            kept.push_back(line.clone());
        }
    }
    for line in kept {
        writer.write_all(&line)?;
    }
    Ok(())
}
