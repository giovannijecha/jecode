use super::{Output, number, string};
use crate::{
    json::{self, Value},
    workspace::{Budget, Error, Workspace},
};

const MAX_FILES: usize = 2000;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 8192;

pub(super) fn search(
    workspace: &Workspace,
    path: &str,
    query: &str,
    limit: usize,
    budget: &Budget<'_>,
) -> Result<Output, Error> {
    let root = path;
    let first = workspace.list(path, budget)?;
    let mut scan = Scan::default();
    let mut pending = Vec::new();
    scan.extend(path, first, &mut pending);
    while let Some((path, directory)) = pending.pop() {
        budget.check()?;
        if directory {
            match workspace.list(&path, budget) {
                Ok(list) => scan.extend(&path, list, &mut pending),
                Err(error @ (Error::Cancelled | Error::Timeout)) => return Err(error),
                Err(_) => {
                    scan.omitted += 1;
                    scan.truncated = true;
                }
            }
            continue;
        }
        if scan.files == MAX_FILES {
            scan.truncated = true;
            break;
        }
        scan.files += 1;
        let text = match workspace.read(&path, budget) {
            Ok(text) => text,
            Err(error @ (Error::Cancelled | Error::Timeout)) => return Err(error),
            Err(_) => {
                scan.omitted += 1;
                continue;
            }
        };
        if text.len() > MAX_BYTES - scan.bytes {
            scan.truncated = true;
            break;
        }
        scan.bytes += text.len();
        for (line, text) in text.lines().enumerate() {
            budget.check()?;
            let Some(column) = text.find(query) else {
                continue;
            };
            let (excerpt, shortened) = excerpt(text, column, query.len());
            let found = json::object([
                ("path", string(&path)),
                ("line", number(line + 1)),
                ("byte_column", number(column + 1)),
                ("text", string(excerpt)),
                ("excerpt_truncated", Value::Bool(shortened)),
            ]);
            let size =
                json::encode(&found, super::MAX_OUTPUT).map_or(super::MAX_OUTPUT, |s| s.len());
            if scan.matches.len() == limit || scan.output_bytes + size > 24 * 1024 {
                scan.truncated = true;
                return Ok(scan.output(root));
            }
            scan.output_bytes += size;
            scan.matches.push(found);
        }
    }
    Ok(scan.output(path))
}

#[derive(Default)]
struct Scan {
    matches: Vec<Value>,
    files: usize,
    bytes: usize,
    entries: usize,
    output_bytes: usize,
    omitted: usize,
    truncated: bool,
}
impl Scan {
    fn extend(
        &mut self,
        path: &str,
        list: crate::workspace::Listing,
        pending: &mut Vec<(String, bool)>,
    ) {
        self.omitted += list.omitted;
        self.truncated |= list.truncated;
        for entry in list.entries.into_iter().rev() {
            if self.entries == MAX_ENTRIES {
                self.truncated = true;
                break;
            }
            self.entries += 1;
            let name = if path == "." {
                entry.name
            } else {
                format!("{path}/{}", entry.name)
            };
            pending.push((name, entry.directory));
        }
    }
    fn output(self, path: &str) -> Output {
        let count = self.matches.len();
        Output::success(
            json::object([
                ("ok", Value::Bool(true)),
                ("path", string(path)),
                ("matches", Value::Array(self.matches)),
                ("files_examined", number(self.files)),
                ("bytes_searched", number(self.bytes)),
                ("omitted", number(self.omitted)),
                ("truncated", Value::Bool(self.truncated)),
            ]),
            format!(
                "{count} matching lines / {} files / {} omitted{}",
                self.files,
                self.omitted,
                if self.truncated { " / limited" } else { "" }
            ),
            self.truncated || self.omitted != 0,
        )
    }
}

fn excerpt(text: &str, column: usize, query: usize) -> (&str, bool) {
    let mut start = column.saturating_sub(120);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let mut end = (column + query + 200).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[start..end], start != 0 || end != text.len())
}
