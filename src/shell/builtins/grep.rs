//! `grep` over files, directories (`-r`) or standard input, with the options
//! models reach for: `-E -F -G -P -i -v -n -c -l -L -q -s -h -H -o -w -x -r`,
//! `-e`, `-m`, `-A -B -C`, `--include`, `--exclude` and `--exclude-dir`.

use super::regex::{self, Regex};
use super::*;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};

#[derive(Default)]
struct Options {
    patterns: Vec<String>,
    extended: bool,
    fixed: bool,
    insensitive: bool,
    invert: bool,
    numbers: bool,
    count: bool,
    listed: Option<bool>,
    quiet: bool,
    silent: bool,
    names: Option<bool>,
    only: bool,
    words: bool,
    whole: bool,
    recursive: bool,
    after: usize,
    before: usize,
    max: Option<usize>,
    include: Vec<String>,
    exclude: Vec<String>,
    exclude_dir: Vec<String>,
    operands: Vec<String>,
}

/// Usage and pattern errors exit with status 2, as in POSIX `grep`.
pub(super) fn grep(shell: &mut Shell, args: &[String], io: &Io) -> Outcome {
    run(shell, args, io).or_else(|error| Ok((2, Vec::new(), format!("{error}\n").into_bytes())))
}

fn run(shell: &mut Shell, args: &[String], io: &Io) -> Outcome {
    let mut options = parse(args)?;
    let pattern = combined(&options);
    let regex =
        Regex::new(&pattern, options.insensitive).map_err(|error| format!("grep: {error}"))?;
    let implicit = options.operands.is_empty();
    if implicit {
        options
            .operands
            .push(if options.recursive { "." } else { "-" }.into());
    }
    let mut files = Vec::new();
    let mut errors = Vec::new();
    for operand in &options.operands {
        collect(shell, &options, operand, implicit, &mut files, &mut errors);
    }
    let names = options
        .names
        .unwrap_or(options.recursive || options.operands.len() > 1);
    let mut writer = &io.stdout;
    let mut selected = false;
    for name in &files {
        let source: Box<dyn Read + '_> = if name == "-" {
            Box::new(&io.stdin)
        } else {
            match fs::File::open(shell.path(name)) {
                // The size at opening bounds the read, so a file that is also
                // the output cannot feed itself.
                Ok(file) => {
                    let size = file.metadata().map_or(u64::MAX, |metadata| metadata.len());
                    Box::new(file.take(size))
                }
                Err(error) => {
                    errors.push(format!("grep: {name}: {error}"));
                    continue;
                }
            }
        };
        let shown = if name == "-" {
            "(standard input)"
        } else {
            name
        };
        match search(&options, &regex, source, shown, names, &mut writer) {
            Ok(found) => selected |= found,
            Err(error) if closed(&error) => {
                shell.broken_output = true;
                return Ok((1, Vec::new(), Vec::new()));
            }
            Err(error) => errors.push(format!("grep: {name}: {error}")),
        }
        if selected && options.quiet {
            break;
        }
    }
    if let Err(error) = writer.flush()
        && closed(&error)
    {
        shell.broken_output = true;
    }
    let status = match (selected, errors.is_empty() || options.quiet && selected) {
        (_, false) => 2,
        (true, true) => 0,
        (false, true) => 1,
    };
    let stderr = if options.silent {
        Vec::new()
    } else {
        errors
            .iter()
            .map(|error| format!("{error}\n"))
            .collect::<String>()
            .into_bytes()
    };
    Ok((status, Vec::new(), stderr))
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut rest = args[1..].iter();
    let mut operands_only = false;
    let number = |value: &str| {
        value
            .parse::<usize>()
            .map_err(|_| format!("grep: invalid number: {value}"))
    };
    while let Some(arg) = rest.next() {
        if operands_only || arg == "-" || !arg.starts_with('-') {
            options.operands.push(arg.clone());
            continue;
        }
        if arg == "--" {
            operands_only = true;
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline) = long
                .split_once('=')
                .map_or((long, None), |(name, value)| (name, Some(value)));
            let mut value = || {
                inline
                    .map(str::to_owned)
                    .or_else(|| rest.next().cloned())
                    .ok_or(format!("grep: --{name} needs a value"))
            };
            match name {
                "include" => options.include.push(value()?),
                "exclude" => options.exclude.push(value()?),
                "exclude-dir" => options.exclude_dir.push(value()?),
                "regexp" => options.patterns.push(value()?),
                "max-count" => options.max = Some(number(&value()?)?),
                "after-context" => options.after = number(&value()?)?,
                "before-context" => options.before = number(&value()?)?,
                "context" => {
                    let lines = number(&value()?)?;
                    (options.after, options.before) = (lines, lines);
                }
                "color" | "colour" | "binary-files" => {}
                "extended-regexp" => options.extended = true,
                "perl-regexp" => {
                    return Err("grep: Perl regular expressions are unsupported".into());
                }
                "fixed-strings" => options.fixed = true,
                "ignore-case" => options.insensitive = true,
                "invert-match" => options.invert = true,
                "line-number" => options.numbers = true,
                "count" => options.count = true,
                "files-with-matches" => options.listed = Some(true),
                "files-without-match" => options.listed = Some(false),
                "quiet" | "silent" => options.quiet = true,
                "no-messages" => options.silent = true,
                "recursive" | "dereference-recursive" => options.recursive = true,
                "only-matching" => options.only = true,
                "word-regexp" => options.words = true,
                "line-regexp" => options.whole = true,
                "no-filename" => options.names = Some(false),
                "with-filename" => options.names = Some(true),
                _ => return Err(format!("grep: unsupported option --{name}")),
            }
            continue;
        }
        let flags: Vec<char> = arg[1..].chars().collect();
        let mut index = 0;
        while index < flags.len() {
            let flag = flags[index];
            index += 1;
            let mut value = || -> Result<String, String> {
                if index < flags.len() {
                    let attached = flags[index..].iter().collect();
                    index = flags.len();
                    Ok(attached)
                } else {
                    rest.next()
                        .cloned()
                        .ok_or(format!("grep: -{flag} needs a value"))
                }
            };
            match flag {
                'E' => options.extended = true,
                'P' => return Err("grep: Perl regular expressions are unsupported".into()),
                'G' => options.extended = false,
                'F' => options.fixed = true,
                'i' | 'y' => options.insensitive = true,
                'v' => options.invert = true,
                'n' => options.numbers = true,
                'c' => options.count = true,
                'l' => options.listed = Some(true),
                'L' => options.listed = Some(false),
                'q' => options.quiet = true,
                's' => options.silent = true,
                'h' => options.names = Some(false),
                'H' => options.names = Some(true),
                'o' => options.only = true,
                'w' => options.words = true,
                'x' => options.whole = true,
                'r' | 'R' => options.recursive = true,
                'I' | 'a' | 'U' => {}
                'e' => options.patterns.push(value()?),
                'm' => options.max = Some(number(&value()?)?),
                'A' => options.after = number(&value()?)?,
                'B' => options.before = number(&value()?)?,
                'C' => {
                    let lines = number(&value()?)?;
                    (options.after, options.before) = (lines, lines);
                }
                digit if digit.is_ascii_digit() => {
                    let lines = number(&flags[index - 1..].iter().collect::<String>())?;
                    index = flags.len();
                    (options.after, options.before) = (lines, lines);
                }
                _ => return Err(format!("grep: unsupported option -{flag}")),
            }
        }
    }
    if options.patterns.is_empty() {
        if options.operands.is_empty() {
            return Err("grep: expected a pattern".into());
        }
        options.patterns.push(options.operands.remove(0));
    }
    Ok(options)
}

/// One extended pattern for every `-e` pattern and line of a pattern.
fn combined(options: &Options) -> String {
    let alternatives: Vec<String> = options
        .patterns
        .iter()
        .flat_map(|pattern| pattern.split('\n'))
        .map(|pattern| {
            let pattern = if options.fixed {
                pattern
                    .chars()
                    .flat_map(|ch| {
                        let meta = "\\.[]()*+?{}|^$".contains(ch);
                        meta.then_some('\\').into_iter().chain([ch])
                    })
                    .collect()
            } else if options.extended {
                pattern.to_owned()
            } else {
                regex::extended(pattern)
            };
            format!("({pattern})")
        })
        .collect();
    let joined = alternatives.join("|");
    if options.whole {
        format!("^({joined})$")
    } else if options.words {
        format!("\\b({joined})\\b")
    } else {
        joined
    }
}

/// Expands an operand into files, descending into directories with `-r`.
fn collect(
    shell: &Shell,
    options: &Options,
    operand: &str,
    implicit: bool,
    files: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    let path = shell.path(operand);
    if operand == "-" || !path.is_dir() {
        files.push(operand.to_owned());
        return;
    }
    if !options.recursive {
        errors.push(format!("grep: {operand}: Is a directory"));
        return;
    }
    let Ok(entries) = fs::read_dir(&path) else {
        errors.push(format!("grep: {operand}: cannot read the directory"));
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        let child = if implicit && operand == "." {
            name.clone()
        } else {
            format!("{}/{name}", operand.trim_end_matches('/'))
        };
        if shell.path(&child).is_dir() {
            if !options
                .exclude_dir
                .iter()
                .any(|glob| super::super::glob::name_matches(glob, &name))
            {
                collect(shell, options, &child, false, files, errors);
            }
        } else if included(options, &name) {
            files.push(child);
        }
    }
}

fn included(options: &Options, name: &str) -> bool {
    let matches = |globs: &[String]| {
        globs
            .iter()
            .any(|glob| super::super::glob::name_matches(glob, name))
    };
    (options.include.is_empty() || matches(&options.include)) && !matches(&options.exclude)
}

/// Searches one input; returns whether any line was selected.
fn search(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_count_zero_does_not_wait_for_standard_input() {
        let (reader, writer) = io::pipe().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let child = std::thread::spawn(move || {
            let options = Options {
                max: Some(0),
                ..Options::default()
            };
            let regex = Regex::new("foo", false).unwrap();
            let mut output = Vec::new();
            sender
                .send(search(
                    &options,
                    &regex,
                    Box::new(reader),
                    "-",
                    false,
                    &mut output,
                ))
                .unwrap();
        });
        let completed = receiver.recv_timeout(std::time::Duration::from_secs(1));
        drop(writer);
        child.join().unwrap();
        assert!(!completed.unwrap().unwrap());
    }
}
