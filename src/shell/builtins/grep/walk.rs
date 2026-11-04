//! Expands `grep` operands into the files to search.

use super::super::*;
use super::Options;

/// Expands an operand into files, descending into directories with `-r`.
pub(super) fn collect(
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
                .any(|glob| crate::shell::glob::name_matches(glob, &name))
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
            .any(|glob| crate::shell::glob::name_matches(glob, name))
    };
    (options.include.is_empty() || matches(&options.include)) && !matches(&options.exclude)
}
