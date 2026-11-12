use super::Layout;
use std::rc::Rc;

#[derive(Debug, Default)]
pub(super) struct Cache {
    source: String,
    columns: usize,
    labels: Vec<String>,
    references: bool,
    layout: Rc<Layout>,
}

impl Cache {
    pub fn get(
        &mut self,
        source: &str,
        cursor: usize,
        columns: usize,
        labels: Vec<String>,
        references: bool,
    ) -> Rc<Layout> {
        let columns = columns.max(4);
        if self.columns != columns
            || self.labels != labels
            || self.references != references
            || self.layout.rows.is_empty()
        {
            self.layout = Rc::new(Layout::new(source, cursor, columns, &labels, references));
            self.columns = columns;
            self.labels = labels;
            self.references = references;
            self.source.clear();
            self.source.push_str(source);
        } else if self.source != source {
            let mut common = self
                .source
                .bytes()
                .zip(source.bytes())
                .take_while(|(before, after)| before == after)
                .count();
            if references && let Some(start) = reference_start(&source.as_bytes()[..common]) {
                common = start;
            }
            // Start before the edit boundary so combining marks and joins can
            // extend the preceding glyph, including at a wrapped row boundary.
            let row = self
                .layout
                .rows
                .partition_point(|row| row.positions[0].0 < common)
                .saturating_sub(1);
            Rc::make_mut(&mut self.layout).reflow(source, row, columns, &self.labels, references);
            self.source.clear();
            self.source.push_str(source);
        }
        if self.layout.cursor != self.layout.cursor_position(cursor) {
            Rc::make_mut(&mut self.layout).move_cursor(cursor);
        }
        Rc::clone(&self.layout)
    }
}

/// The earliest sigil in `prefix` whose reference could reach the edit after it.
/// Closing a quoted reference or editing a reference's end can change chips on
/// earlier wrapped rows. References stay on one line and only `@"…"` crosses
/// spaces; treating other whitespace as part of a word only reflows more.
fn reference_start(prefix: &[u8]) -> Option<usize> {
    let line = prefix
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let word = prefix[line..]
        .iter()
        .rposition(u8::is_ascii_whitespace)
        .map_or(line, |index| line + index + 1);
    let mut quoted = None;
    let mut search = line;
    while let Some(index) = prefix[search..].windows(2).position(|pair| pair == b"@\"") {
        let start = search + index;
        let mut escaped = false;
        let end = prefix[start + 2..].iter().position(|byte| match byte {
            _ if escaped => {
                escaped = false;
                false
            }
            b'\\' => {
                escaped = true;
                false
            }
            b'"' => true,
            _ => false,
        });
        if let Some(end) = end {
            // A completed quoted path cannot change when editing a later word.
            search = start + 3 + end;
        } else {
            quoted = Some(start);
            break;
        }
    }
    let sigil = prefix[word..]
        .iter()
        .position(|byte| matches!(byte, b'$' | b'@'))
        .map(|index| word + index);
    quoted.into_iter().chain(sigil).min()
}

#[cfg(test)]
mod tests {
    use super::reference_start;

    #[test]
    fn closed_quoted_paths_do_not_reflow_unrelated_words_after_them() {
        for prefix in [
            r#"See @"docs/my file.md" then "#,
            r#"See @"docs/a\"b.md" then "#,
            r#"See @"docs/a\\" then "#,
            r#"See @"one.md" and @"two.md" then "#,
        ] {
            assert_eq!(reference_start(prefix.as_bytes()), None, "{prefix}");
        }
    }

    #[test]
    fn unfinished_quotes_and_the_edited_word_still_reflow_from_their_sigil() {
        for prefix in [
            r#"See @"docs/my file"#,
            r#"See @"docs/a\"b"#,
            "See $review,",
            "See @src/file.rs",
        ] {
            assert_eq!(reference_start(prefix.as_bytes()), Some(4), "{prefix}");
        }
        let prefix = r#"See @"one.md" then @"docs/my file"#;
        assert_eq!(
            reference_start(prefix.as_bytes()),
            Some(r#"See @"one.md" then "#.len())
        );
    }
}
