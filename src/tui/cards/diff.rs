use crate::tui::{
    line::Line,
    theme::{BAD, DIFF_ADDED_SURFACE, DIFF_REMOVED_SURFACE, GOOD},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    Removed,
    Added,
}

/// An edit argument is a replacement snippet. Its old and new text are shown
/// in order, without inventing context or file coordinates.
pub(super) fn lines<'a>(old: &'a str, new: &'a str) -> Vec<(Change, &'a str)> {
    old.lines()
        .map(|line| (Change::Removed, line))
        .chain(new.lines().map(|line| (Change::Added, line)))
        .collect()
}

pub(super) fn row(change: Change, line: &str) -> Line {
    match change {
        Change::Removed => Line::new(&format!("- {line}"), BAD).on(DIFF_REMOVED_SURFACE),
        Change::Added => Line::new(&format!("+ {line}"), GOOD).on(DIFF_ADDED_SURFACE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(old: &str, new: &str) -> Vec<String> {
        lines(old, new)
            .into_iter()
            .map(|(change, line)| {
                let mark = match change {
                    Change::Removed => '-',
                    Change::Added => '+',
                };
                format!("{mark}{line}")
            })
            .collect()
    }

    #[test]
    fn replacement_snippets_keep_every_old_and_new_line() {
        assert_eq!(
            marked("a\nb\nc\nd\n", "a\nx\ny\nc\nd\n"),
            ["-a", "-b", "-c", "-d", "+a", "+x", "+y", "+c", "+d"]
        );
        assert_eq!(marked("a\nb\nc", "a\nc"), ["-a", "-b", "-c", "+a", "+c"]);
        assert_eq!(marked("", "new"), ["+new"]);
        assert_eq!(marked("same", "same"), ["-same", "+same"]);
    }

    #[test]
    fn source_order_remains_removals_then_additions() {
        assert_eq!(
            marked("x\none\ntwo\ny", "x\ntwo\none\ny"),
            ["-x", "-one", "-two", "-y", "+x", "+two", "+one", "+y"]
        );
    }

    #[test]
    fn long_replacements_do_not_drop_lines() {
        let old = (0..600).map(|n| format!("old {n}\n")).collect::<String>();
        let new = (0..600).map(|n| format!("new {n}\n")).collect::<String>();
        let changes = lines(&old, &new);
        assert_eq!(changes.len(), 1200);
        assert!(
            changes[..600]
                .iter()
                .all(|(change, _)| *change == Change::Removed)
        );
        assert_eq!(changes[600].0, Change::Added);
    }
}
