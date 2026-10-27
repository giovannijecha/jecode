//! Apply discovery's path filters to saved references, including missing files.
use crate::{
    cancel::Cancellation,
    json::Value,
    path_filter::{Ignore, Pattern},
};
use std::path::Path;

pub(super) struct Selection<'a> {
    scope: &'a str,
    pattern: Option<Pattern>,
    hidden: bool,
    ignored: bool,
    ignore: Ignore,
    cancellation: &'a Cancellation,
}

impl<'a> Selection<'a> {
    pub fn new(root: &Path, scope: &'a str, args: &Value, cancellation: &'a Cancellation) -> Self {
        Self {
            scope,
            pattern: args
                .get("pattern")
                .and_then(Value::as_str)
                .map(Pattern::new),
            hidden: args.get("include_hidden") == Some(&Value::Bool(true)),
            ignored: args.get("include_ignored") == Some(&Value::Bool(true)),
            ignore: Ignore::new(root),
            cancellation,
        }
    }

    pub fn note(&mut self, note: &Value) -> Result<bool, String> {
        let Some(sources) = note.get("sources").and_then(Value::as_array) else {
            return Ok(false);
        };
        for source in sources {
            let Some(path) = source.get("path").and_then(Value::as_str) else {
                continue;
            };
            let scope = if self.scope == "." { "" } else { self.scope };
            let below = if path == scope {
                ""
            } else if scope.is_empty() {
                path
            } else if let Some(below) = path
                .strip_prefix(scope)
                .and_then(|suffix| suffix.strip_prefix('/'))
            {
                below
            } else {
                continue;
            };
            if !below.is_empty() {
                if !self.hidden && below.split('/').any(|part| part.starts_with('.')) {
                    continue;
                }
                if !self.ignored
                    && (below.split('/').any(|part| part == ".git")
                        || self.ignore.ignored_entry(path, false)
                        || self.ignore.ignored_beneath(path, scope, false))
                {
                    continue;
                }
            }
            if let Some(pattern) = &self.pattern
                && !pattern.matches(path, self.cancellation)?
            {
                continue;
            }
            return Ok(true);
        }
        Ok(false)
    }
}
