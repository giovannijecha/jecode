//! A file change retained for the interface, separate from the model's tool result.

pub(crate) const MAX_DIFF_BYTES: usize = 1024 * 1024;
const MAX_DIFF_LINES: usize = 20_000;
const MAX_ALIGNMENT_CELLS: usize = 2_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Context,
    Removed,
    Added,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
    pub no_newline: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub added: usize,
    pub removed: usize,
    pub created: bool,
    pub hunks: Vec<Vec<Entry>>,
    pub ending_change: Option<String>,
    pub final_newline: Option<bool>,
    /// A multi-file edit: each changed file's own change by the path the call
    /// named, in call order. The counts above are their sums; `hunks` is empty.
    pub files: Vec<(String, Change)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ending {
    None,
    Lf,
    CrLf,
}

struct Line<'a> {
    text: &'a str,
    ending: Ending,
}

fn lines(text: &str) -> Vec<Line<'_>> {
    text.split_inclusive('\n')
        .map(|line| {
            if let Some(body) = line.strip_suffix("\r\n") {
                Line {
                    text: body,
                    ending: Ending::CrLf,
                }
            } else if let Some(body) = line.strip_suffix('\n') {
                Line {
                    text: body,
                    ending: Ending::Lf,
                }
            } else {
                Line {
                    text: line,
                    ending: Ending::None,
                }
            }
        })
        .filter(|line| !line.text.is_empty() || line.ending != Ending::None)
        .collect()
}

fn style(lines: &[Line<'_>]) -> Option<&'static str> {
    let lf = lines.iter().any(|line| line.ending == Ending::Lf);
    let crlf = lines.iter().any(|line| line.ending == Ending::CrLf);
    match (lf, crlf) {
        (true, true) => Some("mixed"),
        (true, false) => Some("LF"),
        (false, true) => Some("CRLF"),
        (false, false) => None,
    }
}

fn scores(a: &[Line<'_>], b: &[Line<'_>], reverse: bool) -> Vec<usize> {
    let mut previous = vec![0; b.len() + 1];
    let mut current = vec![0; b.len() + 1];
    for left in 0..a.len() {
        let a_line = if reverse {
            &a[a.len() - 1 - left]
        } else {
            &a[left]
        };
        for right in 0..b.len() {
            let b_line = if reverse {
                &b[b.len() - 1 - right]
            } else {
                &b[right]
            };
            current[right + 1] = if a_line.text == b_line.text {
                previous[right] + 1
            } else {
                previous[right + 1].max(current[right])
            };
        }
        std::mem::swap(&mut previous, &mut current);
        current.fill(0);
    }
    previous
}

fn matches(
    old: &[Line<'_>],
    new: &[Line<'_>],
    old_start: usize,
    new_start: usize,
    result: &mut Vec<(usize, usize)>,
) {
    if old.is_empty() || new.is_empty() {
        return;
    }
    if old.len() == 1 {
        if let Some(index) = new.iter().position(|line| line.text == old[0].text) {
            result.push((old_start, new_start + index));
        }
        return;
    }
    if new.len() == 1 {
        if let Some(index) = old.iter().position(|line| line.text == new[0].text) {
            result.push((old_start + index, new_start));
        }
        return;
    }
    let middle = old.len() / 2;
    let boundary = {
        let left = scores(&old[..middle], new, false);
        let right = scores(&old[middle..], new, true);
        (0..=new.len())
            .max_by_key(|&index| left[index] + right[new.len() - index])
            .unwrap()
    };
    matches(
        &old[..middle],
        &new[..boundary],
        old_start,
        new_start,
        result,
    );
    matches(
        &old[middle..],
        &new[boundary..],
        old_start + middle,
        new_start + boundary,
        result,
    );
}

impl Change {
    pub fn redacted(&self, redactor: &crate::redact::Redactor) -> Self {
        let mut change = self.clone();
        for entry in change.hunks.iter_mut().flatten() {
            entry.text = redactor.text(&entry.text);
        }
        for (path, file) in &mut change.files {
            *path = redactor.text(path);
            *file = file.redacted(redactor);
        }
        change
    }

    /// One change for a call that changed several files, or `None` when no
    /// file has a diff to show.
    pub fn several(files: Vec<(String, Change)>) -> Option<Self> {
        (!files.is_empty()).then(|| Self {
            added: files.iter().map(|(_, change)| change.added).sum(),
            removed: files.iter().map(|(_, change)| change.removed).sum(),
            created: false,
            hunks: Vec::new(),
            ending_change: None,
            final_newline: None,
            files,
        })
    }

    /// The change to one file of a multi-file edit.
    pub fn file(&self, path: &str) -> Option<&Self> {
        self.files
            .iter()
            .find(|(candidate, _)| candidate == path)
            .map(|(_, change)| change)
    }

    pub fn between(before: Option<&str>, after: &str) -> Option<Self> {
        if before.is_some_and(|text| text.len() > MAX_DIFF_BYTES) || after.len() > MAX_DIFF_BYTES {
            return None;
        }
        let old = lines(before.unwrap_or(""));
        let new = lines(after);
        if old.len().max(new.len()) > MAX_DIFF_LINES {
            return None;
        }
        let mut paired = Vec::new();
        let mut prefix = 0;
        while prefix < old.len() && prefix < new.len() && old[prefix].text == new[prefix].text {
            paired.push((prefix, prefix));
            prefix += 1;
        }
        let mut old_end = old.len();
        let mut new_end = new.len();
        while old_end > prefix && new_end > prefix && old[old_end - 1].text == new[new_end - 1].text
        {
            old_end -= 1;
            new_end -= 1;
        }
        let old_middle = &old[prefix..old_end];
        let new_middle = &new[prefix..new_end];
        let old_texts: std::collections::HashSet<_> =
            old_middle.iter().map(|line| line.text).collect();
        if new_middle.iter().any(|line| old_texts.contains(line.text)) {
            if old_middle.len().saturating_mul(new_middle.len()) > MAX_ALIGNMENT_CELLS {
                return None;
            }
            matches(old_middle, new_middle, prefix, prefix, &mut paired);
        }
        for index in 0..old.len() - old_end {
            paired.push((old_end + index, new_end + index));
        }
        let changed_endings = paired.iter().any(|&(previous, current)| {
            old[previous].ending != new[current].ending
                && old[previous].ending != Ending::None
                && new[current].ending != Ending::None
        });

        let mut entries = Vec::new();
        let mut old_at = 0;
        let mut new_at = 0;
        for (old_match, new_match) in paired
            .into_iter()
            .chain(std::iter::once((old.len(), new.len())))
        {
            while old_at < old_match {
                entries.push(Entry {
                    kind: Kind::Removed,
                    old_line: Some(old_at + 1),
                    new_line: None,
                    text: old[old_at].text.into(),
                    no_newline: old_at + 1 == old.len() && old[old_at].ending == Ending::None,
                });
                old_at += 1;
            }
            while new_at < new_match {
                entries.push(Entry {
                    kind: Kind::Added,
                    old_line: None,
                    new_line: Some(new_at + 1),
                    text: new[new_at].text.into(),
                    no_newline: new_at + 1 == new.len() && new[new_at].ending == Ending::None,
                });
                new_at += 1;
            }
            if old_match < old.len() {
                entries.push(Entry {
                    kind: Kind::Context,
                    old_line: Some(old_at + 1),
                    new_line: Some(new_at + 1),
                    text: old[old_at].text.into(),
                    no_newline: new_at + 1 == new.len() && new[new_at].ending == Ending::None,
                });
                old_at += 1;
                new_at += 1;
            }
        }
        let added = entries
            .iter()
            .filter(|entry| entry.kind == Kind::Added)
            .count();
        let removed = entries
            .iter()
            .filter(|entry| entry.kind == Kind::Removed)
            .count();
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            if entry.kind == Kind::Context {
                continue;
            }
            let start = index.saturating_sub(3);
            let end = (index + 4).min(entries.len());
            if let Some(last) = ranges.last_mut()
                && start <= last.1
            {
                last.1 = end;
            } else {
                ranges.push((start, end));
            }
        }
        let hunks = ranges
            .into_iter()
            .map(|(start, end)| entries[start..end].to_vec())
            .collect();
        let ending_change = before.and_then(|_| {
            let (Some(from), Some(to)) = (style(&old), style(&new)) else {
                return None;
            };
            if from != to {
                return Some(format!("{from} → {to}"));
            }
            (from == "mixed" && changed_endings).then(|| "mixed line endings updated".into())
        });
        let final_newline = before.and_then(|old| {
            let previous = old.ends_with('\n');
            let current = after.ends_with('\n');
            (previous != current).then_some(current)
        });
        Some(Self {
            added,
            removed,
            created: before.is_none(),
            hunks,
            ending_change,
            final_newline,
            files: Vec::new(),
        })
    }
}

mod saved;

#[cfg(test)]
mod tests;
