//! Searches one input line by line and prints what `grep` selected. Lines are
//! bytes: they are matched as UTF-8 where valid and printed as they were read.

use super::super::regex::{self, Regex};
use super::super::*;
use super::options::{Binary, Options};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};

const CHUNK: usize = 64 * 1024;

/// Reads lines in chunks. Once a chunk holds a NUL the input is binary and a NUL
/// also ends a line, as in GNU grep, so a binary file without newlines is not
/// held as one line.
pub(super) struct Lines<'a> {
    reader: BufReader<Box<dyn Read + 'a>>,
    detect: bool,
    binary: bool,
    /// An empty line is not selected, so a run of NUL bytes needs no search.
    skip_nuls: bool,
}

impl<'a> Lines<'a> {
    pub(super) fn new(source: Box<dyn Read + 'a>, detect: bool, skip_nuls: bool) -> Self {
        Self {
            reader: BufReader::with_capacity(CHUNK, source),
            detect,
            binary: false,
            skip_nuls,
        }
    }

    pub(super) fn binary(&self) -> bool {
        self.binary
    }

    /// Replaces `line` with the next line, without its terminator; `false` at
    /// the end of the input.
    pub(super) fn next(&mut self, line: &mut Vec<u8>) -> io::Result<bool> {
        line.clear();
        loop {
            let fresh = self.reader.buffer().is_empty();
            let chunk = self.reader.fill_buf()?;
            if chunk.is_empty() {
                return Ok(!line.is_empty());
            }
            self.binary |= fresh && self.detect && chunk.contains(&0);
            if self.binary && self.skip_nuls && line.is_empty() {
                let nuls = chunk.iter().take_while(|byte| **byte == 0).count();
                if nuls > 0 {
                    self.reader.consume(nuls);
                    continue;
                }
            }
            let binary = self.binary;
            let end = chunk
                .iter()
                .position(|byte| *byte == b'\n' || binary && *byte == 0);
            let taken = end.map_or(chunk.len(), |end| end);
            line.extend_from_slice(&chunk[..taken]);
            self.reader.consume(taken + usize::from(end.is_some()));
            if end.is_some() {
                return Ok(true);
            }
        }
    }
}

/// Searches one input; returns whether any line was selected.
pub(super) fn search(
    options: &Options,
    regex: &Regex,
    source: Box<dyn Read + '_>,
    name: &str,
    names: bool,
    writer: &mut dyn Write,
) -> io::Result<bool> {
    let empty_selected = regex.find(b"", 0).is_some() != options.invert;
    let mut lines = Lines::new(source, options.binary != Binary::Text, !empty_selected);
    let mut bytes = Vec::new();
    let mut before: VecDeque<(usize, Vec<u8>)> = VecDeque::new();
    let (mut number, mut count, mut after) = (0usize, 0usize, 0usize);
    let mut last: Option<usize> = None;
    let lines_shown = !options.quiet && !options.count && options.listed.is_none();
    loop {
        if options.max.is_some_and(|max| count >= max) && after == 0 {
            break;
        }
        if !lines.next(&mut bytes)? {
            break;
        }
        number += 1;
        if lines.binary() && options.binary == Binary::WithoutMatch {
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
            if !lines_shown {
                continue;
            }
            if lines.binary() {
                writeln!(writer, "Binary file {name} matches")?;
                return Ok(true);
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
        if !lines_shown || lines.binary() {
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
    Ok(count > 0)
}

/// A line without the carriage returns of a CRLF ending.
fn content(line: &[u8]) -> &[u8] {
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
