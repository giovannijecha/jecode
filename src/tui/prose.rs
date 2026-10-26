//! Assistant voice uses a neutral marker and a two-cell hanging gutter.
use super::{line::Line, markdown, theme::BODY};

fn gutter(lines: &mut [Line], start: usize, columns: usize) {
    for (index, line) in lines.iter_mut().enumerate() {
        *line = std::mem::take(line)
            .gutter(if start + index == 0 { "● " } else { "  " }, BODY)
            .shortened(columns);
    }
}

pub(super) fn render(value: &str, columns: usize) -> Vec<Line> {
    Rows::new(value, columns).flatten().collect()
}

pub(super) struct Rows {
    markdown: markdown::Rows,
    rendered: usize,
    columns: usize,
}

impl Rows {
    pub fn new(value: &str, columns: usize) -> Self {
        Self {
            markdown: markdown::Rows::new(value, columns.saturating_sub(2)),
            rendered: 0,
            columns,
        }
    }
}

impl Iterator for Rows {
    type Item = Vec<Line>;
    fn next(&mut self) -> Option<Self::Item> {
        let mut lines = self.markdown.next()?;
        gutter(&mut lines, self.rendered, self.columns);
        self.rendered += lines.len();
        Some(lines)
    }
}

pub(super) struct Stream {
    markdown: markdown::Stream,
    columns: usize,
}

impl Stream {
    pub fn new(value: &str, columns: usize) -> Self {
        Self {
            markdown: markdown::Stream::new(value, columns.saturating_sub(2)),
            columns,
        }
    }
    pub fn update(&mut self, value: &str, columns: usize) -> Option<usize> {
        self.columns = columns;
        self.markdown.update(value, columns.saturating_sub(2))
    }
    pub fn advance(&mut self, lines: &mut Vec<Line>, max_rows: usize) {
        let start = lines.len();
        self.markdown.advance(lines, max_rows);
        // A transcript separator is outside the message's source rows.
        let prefix = usize::from(lines.first().is_some_and(|line| line.origin.is_none()));
        gutter(
            &mut lines[start..],
            start.saturating_sub(prefix),
            self.columns,
        );
    }
    pub fn ready(&self) -> bool {
        self.markdown.ready()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_has_one_neutral_dot_and_every_paragraph_hangs_two_cells() {
        for columns in [40, 80] {
            let lines = render(
                "A long assistant paragraph that wraps onto another line at narrow widths.\n\nThe second paragraph keeps the same indent.",
                columns,
            );
            assert!(lines[0].plain().starts_with("● A long"));
            assert_eq!(lines[0].spans[0].style, BODY);
            assert!(lines[1..].iter().all(|line| line.plain().starts_with("  ")));
            assert_eq!(
                lines
                    .iter()
                    .filter(|line| line.plain().starts_with("● "))
                    .count(),
                1
            );
            assert!(
                lines
                    .iter()
                    .all(|line| super::super::text::cells(&line.plain()) <= columns)
            );
        }
    }

    #[test]
    fn streamed_markdown_matches_settled_voice_through_rewrites_and_resizes() {
        let mut stream = Stream::new("First", 40);
        let mut lines = vec![Line::default()];
        for (value, columns) in [
            ("First", 40),
            ("First paragraph.\n\nSecond **paragraph**.", 40),
            ("| Name | Value |\n| --- | --- |\n| item | ready |", 80),
            ("```js\nconst value = 42;\n```\n\nDone.", 40),
        ] {
            if let Some(keep) = stream.update(value, columns) {
                lines.truncate(keep.max(1));
            }
            for _ in 0..100 {
                stream.advance(&mut lines, 1);
                if stream.ready() {
                    break;
                }
            }
            assert!(stream.ready());
            assert_eq!(&lines[1..], render(value, columns));
        }
    }
}
