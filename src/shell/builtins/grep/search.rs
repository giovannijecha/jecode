//! Searches one input line by line and prints what `grep` selected. Lines are
//! bytes: they are matched as UTF-8 where valid and printed as they were read.

use super::super::regex::{self, Regex};
use super::super::*;
use super::options::{Binary, Options};
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
    let mut before: VecDeque<(usize, Vec<u8>)> = VecDeque::new();
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
        binary |= options.binary != Binary::Text && bytes.contains(&0);
        if binary && options.binary == Binary::WithoutMatch {
            count = 0;
            break;
        }
        let line = content(&bytes);
        let hit = regex.find(line, 0).is_some() != options.invert;
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
                terminated(writer, &text)?;
            }
            if options.only && !options.invert {
                let mut from = 0;
                while let Some((start, end)) = regex.find(line, from) {
                    if end > start {
                        prefix(writer, options, name, names, number, ':')?;
                        terminated(writer, &line[start..end])?;
                        from = end;
                    } else {
                        from = regex::step(line, end);
                    }
                }
            } else if !options.only {
                prefix(writer, options, name, names, number, ':')?;
                terminated(writer, line)?;
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
            terminated(writer, line)?;
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

/// A line without its line ending.
fn content(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let end = line
        .iter()
        .rposition(|byte| *byte != b'\r')
        .map_or(0, |last| last + 1);
    &line[..end]
}

fn terminated(writer: &mut dyn Write, text: &[u8]) -> io::Result<()> {
    writer.write_all(text)?;
    writer.write_all(b"\n")
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
