//! Expands `grep` operands into the files to search.

use super::super::*;
use super::Options;
use std::path::PathBuf;

/// Expands an operand into files, descending into directories with `-r`. A
/// link named as an operand is followed; links met during the walk are skipped
/// unless `-R` asked to follow them.
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
    let mut walk = Walk {
        shell,
        options,
        files,
        errors,
        ancestors: Vec::new(),
    };
    walk.directory(operand, implicit);
}

struct Walk<'a> {
    shell: &'a Shell,
    options: &'a Options,
    files: &'a mut Vec<String>,
    errors: &'a mut Vec<String>,
    /// The directories being walked, so `-R` can stop at a link back to one.
    ancestors: Vec<PathBuf>,
}

impl Walk<'_> {
    fn directory(&mut self, operand: &str, implicit: bool) {
        let path = self.shell.path(operand);
        let canonical = self
            .options
            .follow
            .then(|| path.canonicalize().ok())
            .flatten();
        if let Some(canonical) = &canonical {
            if self.ancestors.contains(canonical) {
                return;
            }
            self.ancestors.push(canonical.clone());
        }
        let Ok(entries) = fs::read_dir(&path) else {
            self.errors
                .push(format!("grep: {operand}: cannot read the directory"));
            return;
        };
        let mut children: Vec<(String, fs::FileType)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let kind = entry.file_type().ok()?;
                Some((entry.file_name().to_string_lossy().into_owned(), kind))
            })
            .collect();
        children.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, kind) in children {
            let child = if implicit && operand == "." {
                name.clone()
            } else {
                format!("{}/{name}", operand.trim_end_matches('/'))
            };
            let target = if kind.is_symlink() {
                if !self.options.follow {
                    continue;
                }
                // A dangling link stays a file, so opening it reports the error.
                match fs::metadata(self.shell.path(&child)) {
                    Ok(metadata) if metadata.is_dir() => Target::Directory,
                    Ok(metadata) if !metadata.is_file() => Target::Other,
                    _ => Target::File,
                }
            } else if kind.is_dir() {
                Target::Directory
            } else if kind.is_file() {
                Target::File
            } else {
                Target::Other
            };
            match target {
                Target::Directory => {
                    if !self
                        .options
                        .exclude_dir
                        .iter()
                        .any(|glob| crate::shell::glob::name_matches(glob, &name))
                    {
                        self.directory(&child, false);
                    }
                }
                Target::File => {
                    if included(self.options, &name) {
                        self.files.push(child);
                    }
                }
                // A device or pipe found in a tree could block forever.
                Target::Other => {}
            }
        }
        if canonical.is_some() {
            self.ancestors.pop();
        }
    }
}

enum Target {
    Directory,
    File,
    Other,
}

fn included(options: &Options, name: &str) -> bool {
    let matches = |globs: &[String]| {
        globs
            .iter()
            .any(|glob| crate::shell::glob::name_matches(glob, name))
    };
    (options.include.is_empty() || matches(&options.include)) && !matches(&options.exclude)
}
