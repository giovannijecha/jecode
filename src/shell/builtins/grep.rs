//! `grep` over files, directories (`-r`) or standard input, with the options
//! models reach for: `-E -F -G -P -i -v -n -c -l -L -q -s -h -H -o -w -x -r`,
//! `-e`, `-m`, `-A -B -C`, `--include`, `--exclude` and `--exclude-dir`.

#[path = "grep/options.rs"]
mod options;
#[path = "grep/search.rs"]
mod search;
#[path = "grep/walk.rs"]
mod walk;

use super::regex::{self, Regex};
use super::*;
use options::{Options, parse};
use search::search;
use std::io::Read;
use walk::collect;

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

#[cfg(test)]
#[path = "grep/tests.rs"]
mod tests;
