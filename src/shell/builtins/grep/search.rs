//! Searches one input line by line and prints what `grep` selected.

use super::super::regex::Regex;
use super::super::*;
use super::Options;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};

/// Searches one input; returns whether any line was selected.
pub(super) fn search(
    options: &Options,
    regex: &Regex,
    source: Box<dyn Read + '_>,
    name: &str,
    names: bool,
    writer: &mut dyn Write,
) -> io::Result<bool> {
    let mut reader = BufReader::new(source);
    let mut bytes = Vec::new();
    let mut before: VecDeque<(usize, String)> = VecDeque::new();
    let (mut number, mut count, mut after, mut binary) = (0usize, 0usize, 0usize, false);
    let mut last: Option<usize> = None;
    let lines_shown = !options.quiet && !options.count && options.listed.is_none();
    loop {
        if options.max.is_some_and(|max| count >= max) && after == 0 {
            break;
        }
        bytes.clear();
        if reader.read_until(b'\n', &mut bytes)? == 0 {
            break;
        }
        number += 1;
        binary |= bytes.contains(&0);
        let line = String::from_utf8_lossy(&bytes);
        let line = line.trim_end_matches('\n').trim_end_matches('\r');
        let chars: Vec<char> = line.chars().collect();
        let hit = regex.find(&chars, 0).is_some() != options.invert;
        if hit && options.max.is_none_or(|max| count < max) {
            count += 1;
            if options.quiet {
                return Ok(true);
            }
            if !lines_shown || binary {
                continue;
            }
            if last.is_some_and(|last| number > last + before.len() + 1)
                && (options.before > 0 || options.after > 0)
            {
                writer.write_all(b"--\n")?;
            }
            for (line_number, text) in before.drain(..) {
                prefix(writer, options, name, names, line_number, '-')?;
                writeln!(writer, "{text}")?;
            }
            if options.only && !options.invert {
                let mut from = 0;
                while let Some((start, end)) = regex.find(&chars, from) {
                    if end > start {
                        prefix(writer, options, name, names, number, ':')?;
                        writeln!(writer, "{}", chars[start..end].iter().collect::<String>())?;
                    }
                    from = end.max(start + 1);
                    if from > chars.len() {
                        break;
                    }
                }
            } else if !options.only {
                prefix(writer, options, name, names, number, ':')?;
                writeln!(writer, "{line}")?;
            }
            last = Some(number);
            after = options.after;
            continue;
        }
        if !lines_shown || binary {
            continue;
        }
        if after > 0 {
            after -= 1;
            prefix(writer, options, name, names, number, '-')?;
            writeln!(writer, "{line}")?;
            last = Some(number);
        } else if options.before > 0 {
            if before.len() == options.before {
                before.pop_front();
            }
            before.push_back((number, line.to_owned()));
        }
    }
    if options.count {
        if names {
            write!(writer, "{name}:")?;
        }
        writeln!(writer, "{count}")?;
    }
    if options.listed == Some(count > 0) {
        writeln!(writer, "{name}")?;
    }
    if binary && count > 0 && lines_shown {
        writeln!(writer, "Binary file {name} matches")?;
    }
    Ok(count > 0)
}

fn prefix(
    writer: &mut dyn Write,
    options: &Options,
    name: &str,
    names: bool,
    number: usize,
    separator: char,
) -> io::Result<()> {
    if names {
        write!(writer, "{name}{separator}")?;
    }
    if options.numbers {
        write!(writer, "{number}{separator}")?;
    }
    Ok(())
}
