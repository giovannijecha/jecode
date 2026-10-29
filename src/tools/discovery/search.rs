use super::{
    boolean,
    page::Page,
    text::{Failure, Line, Lines, Needle},
    walk::Walk,
};
use crate::{cancel::Cancellation, json::Value, tools::optional_number};
use std::{collections::VecDeque, fs::File, io::BufReader, path::Path};

struct Match {
    path: String,
    line: Line,
    before: Vec<Value>,
    after: Vec<Value>,
}

impl Match {
    fn value(self) -> Value {
        Value::object([
            ("path", Value::string(self.path)),
            ("line", Value::number(self.line.number)),
            ("text", Value::string(self.line.text)),
            ("byte_offset", Value::number(self.line.match_byte.unwrap())),
            ("text_truncated", Value::Bool(self.line.truncated)),
            ("before", Value::Array(self.before)),
            ("after", Value::Array(self.after)),
        ])
    }
}

pub(super) fn execute(
    root: &Path,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let query = super::super::required_string(arguments, "query")?;
    if query.is_empty() || query.contains(['\r', '\n', '\0']) {
        return Err("query must be nonempty single-line literal text without NUL bytes".into());
    }
    let sensitive = boolean(arguments, "case_sensitive", true)?;
    let needle = Needle::new(query, sensitive);
    let context = optional_number(arguments, "context", 2, 0, 10)?;
    let mut page = Page::new(arguments, 100, 1000)?;
    let mut walk = Walk::new(root, arguments, true, true, cancellation)?;
    let mut searched = 0;
    'files: while let Some(entry) = walk.next()? {
        match entry.kind {
            "directory" => continue,
            "symlink" => {
                walk.skipped.links += 1;
                continue;
            }
            "other" => {
                walk.skipped.other += 1;
                continue;
            }
            _ => {}
        }
        if !walk.selected(&entry)? {
            continue;
        }
        let file = match File::open(&entry.absolute) {
            Ok(file) => file,
            Err(error) => {
                walk.error(&entry.relative, error);
                continue;
            }
        };
        searched += 1;
        let mut lines = Lines::new(BufReader::new(file));
        let mut before: VecDeque<Value> = VecDeque::new();
        let mut pending: VecDeque<Match> = VecDeque::new();
        let mut valid_lines = 0;
        loop {
            let line = match lines.next(&needle, cancellation) {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(Failure::Error(error)) if cancellation.requested() => return Err(error),
                Err(error) => {
                    let io_error = matches!(&error, Failure::Error(_));
                    let message = match error {
                        Failure::Binary => {
                            walk.skipped.binary += 1;
                            "File contains binary data".into()
                        }
                        Failure::Encoding => {
                            walk.skipped.non_utf8 += 1;
                            "File is not UTF-8 text".into()
                        }
                        Failure::Error(error) => error,
                    };
                    // A non-text file is outside the text scope. A late failure can
                    // leave valid earlier matches, so its unsearched tail needs a warning.
                    if valid_lines > 0 || io_error {
                        walk.error(&entry.relative, message);
                    }
                    break;
                }
            };
            valid_lines += 1;
            let value = line.value();
            for matched in &mut pending {
                matched.after.push(value.clone());
            }
            while pending
                .front()
                .is_some_and(|matched| matched.after.len() >= context)
            {
                if !page.offer(pending.pop_front().unwrap().value())? {
                    break 'files;
                }
            }
            if line.match_byte.is_some() {
                let matched = Match {
                    path: entry.relative.clone(),
                    line,
                    before: before.iter().cloned().collect(),
                    after: Vec::new(),
                };
                if context == 0 {
                    if !page.offer(matched.value())? {
                        break 'files;
                    }
                } else {
                    pending.push_back(matched);
                }
            }
            if context > 0 {
                before.push_back(value);
                if before.len() > context {
                    before.pop_front();
                }
            }
        }
        for matched in pending {
            if !page.offer(matched.value())? {
                break 'files;
            }
        }
    }
    page.finish(
        "matches",
        &walk,
        [("files_searched", Value::number(searched))],
    )
}
