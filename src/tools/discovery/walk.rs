use super::{boolean, string};
use crate::{
    cancel::Cancellation,
    json::Value,
    path_filter::{Ignore, Pattern},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct Skipped {
    pub hidden: usize,
    pub ignored: usize,
    pub links: usize,
    pub binary: usize,
    pub non_utf8: usize,
    pub other: usize,
}

impl Skipped {
    pub fn value(&self) -> Value {
        Value::object([
            ("hidden", Value::number(self.hidden)),
            ("ignored", Value::number(self.ignored)),
            ("links", Value::number(self.links)),
            ("binary", Value::number(self.binary)),
            ("non_utf8", Value::number(self.non_utf8)),
            ("other", Value::number(self.other)),
        ])
    }
}

pub(super) struct Entry {
    pub absolute: PathBuf,
    pub relative: String,
    pub kind: &'static str,
    pub depth: usize,
    pub size: u64,
}

struct Pending {
    absolute: PathBuf,
    depth: usize,
    explicit: bool,
}

pub(super) struct Walk<'a> {
    root: &'a Path,
    cancellation: &'a Cancellation,
    pending: Vec<Pending>,
    ignore: Ignore,
    pattern: Option<Pattern>,
    include_hidden: bool,
    include_ignored: bool,
    recursive: bool,
    pub scope: String,
    pub skipped: Skipped,
    pub errors: Vec<Value>,
    pub errors_count: usize,
    error_bytes: usize,
}

impl<'a> Walk<'a> {
    pub fn new(
        root: &'a Path,
        arguments: &Value,
        recursive: bool,
        file_allowed: bool,
        cancellation: &'a Cancellation,
    ) -> Result<Self, String> {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let requested = string(arguments, "path", ".")?;
        if requested.is_empty() {
            return Err("path must not be empty".into());
        }
        let absolute = super::super::files::existing_path(root, requested)?;
        let metadata = fs::metadata(&absolute).map_err(|error| error.to_string())?;
        if !(metadata.is_dir() || file_allowed && metadata.is_file()) {
            return Err(if file_allowed {
                "path must refer to a regular file or directory"
            } else {
                "list path must refer to a directory"
            }
            .into());
        }
        let scope = relative(root, &absolute)?;
        let pattern = match arguments.get("pattern") {
            None => None,
            Some(Value::String(value)) if !value.is_empty() && !value.contains('\0') => {
                Some(Pattern::new(value))
            }
            _ => return Err("pattern must be a nonempty path glob".into()),
        };
        Ok(Self {
            root,
            cancellation,
            pending: vec![Pending {
                absolute,
                depth: 0,
                explicit: true,
            }],
            ignore: Ignore::new(root),
            pattern,
            include_hidden: boolean(arguments, "include_hidden", false)?,
            include_ignored: boolean(arguments, "include_ignored", false)?,
            recursive,
            scope,
            skipped: Skipped::default(),
            errors: Vec::new(),
            errors_count: 0,
            error_bytes: 0,
        })
    }

    pub fn selected(&self, entry: &Entry) -> Result<bool, String> {
        match &self.pattern {
            Some(pattern) => pattern.matches(&entry.relative, self.cancellation),
            None => Ok(true),
        }
    }

    pub fn error(&mut self, path: &str, error: impl ToString) {
        self.errors_count += 1;
        let error = Value::object([
            ("path", Value::string(path)),
            ("error", Value::string(error.to_string())),
        ]);
        let size = error.encode().len();
        if self.error_bytes + size <= 8 * 1024 {
            self.error_bytes += size;
            self.errors.push(error);
        }
    }

    pub fn next(&mut self) -> Result<Option<Entry>, String> {
        while let Some(pending) = self.pending.pop() {
            if self.cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            let path = match relative(self.root, &pending.absolute) {
                Ok(path) => path,
                Err(error) => {
                    self.skipped.non_utf8 += 1;
                    self.error(&pending.absolute.to_string_lossy(), error);
                    continue;
                }
            };
            let metadata = match fs::symlink_metadata(&pending.absolute) {
                Ok(metadata) => metadata,
                Err(error) => {
                    self.error(&path, error);
                    continue;
                }
            };
            let linked = crate::disk::linked(&metadata);
            let directory = metadata.is_dir() && !linked;
            let name = pending
                .absolute
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if !pending.explicit {
                if !self.include_hidden && name.starts_with('.') {
                    self.skipped.hidden += 1;
                    continue;
                }
                if !self.include_ignored
                    && (name == ".git" || self.ignore.ignored_entry(&path, directory))
                {
                    self.skipped.ignored += 1;
                    continue;
                }
            }
            if directory && (pending.explicit || self.recursive) {
                self.descend(&pending.absolute, &path, pending.depth);
            }
            if pending.explicit && directory {
                continue;
            }
            return Ok(Some(Entry {
                absolute: pending.absolute,
                relative: path,
                kind: if linked {
                    "symlink"
                } else if directory {
                    "directory"
                } else if metadata.is_file() {
                    "file"
                } else {
                    "other"
                },
                depth: pending.depth,
                size: metadata.len(),
            }));
        }
        if self.cancellation.requested() {
            Err("Operation cancelled".into())
        } else {
            Ok(None)
        }
    }

    fn descend(&mut self, absolute: &Path, relative: &str, depth: usize) {
        let directory = match fs::read_dir(absolute) {
            Ok(directory) => directory,
            Err(error) => {
                self.error(relative, error);
                return;
            }
        };
        let mut children = Vec::new();
        for entry in directory {
            if self.cancellation.requested() {
                break;
            }
            match entry {
                Ok(entry) => children.push(entry.path()),
                Err(error) => self.error(relative, error),
            }
        }
        self.ignore.visit(
            if relative == "." { "" } else { relative },
            children
                .iter()
                .any(|path| path.file_name().is_some_and(|name| name == ".gitignore")),
        );
        children.sort();
        self.pending
            .extend(children.into_iter().rev().map(|absolute| Pending {
                absolute,
                depth: depth + 1,
                explicit: false,
            }));
    }
}

fn relative(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Path is outside the working directory")?;
    let relative = relative.to_str().ok_or("Path is not UTF-8")?.to_owned();
    #[cfg(windows)]
    let relative = relative.replace('\\', "/");
    if relative.is_empty() {
        Ok(".".into())
    } else {
        Ok(relative)
    }
}
