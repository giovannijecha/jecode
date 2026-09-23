//! Append-only corrections for terminal scrollback that cannot be rewritten.

const MAX_EDITS: usize = 32;

struct Edit<'a> {
    start: usize,
    end: usize,
    replacement: &'a str,
}

pub(super) fn note(streamed: &str, final_text: &str) -> String {
    let mut result =
        String::from("Final response correction / the streamed text above is provisional");
    for edit in edits(streamed, final_text) {
        let (line, character) = position(streamed, edit.start);
        let removed = streamed[edit.start..edit.end].chars().count();
        if removed == 0 && edit.replacement == "\n" {
            result.push_str(&format!(
                "\nAt line {line}, character {character}: insert a line break."
            ));
        } else if removed == 0 && edit.replacement == "\n\n" {
            result.push_str(&format!(
                "\nAt line {line}, character {character}: insert a paragraph break."
            ));
        } else if removed == 0
            && let Some(text) = edit.replacement.strip_suffix('\n')
            && !text.contains('\n')
        {
            result.push_str(&format!(
                "\nAt line {line}, character {character}: insert {text:?} followed by a line break."
            ));
        } else if removed == 0 {
            result.push_str(&format!(
                "\nAt line {line}, character {character}: insert {:?}.",
                edit.replacement
            ));
        } else {
            result.push_str(&format!(
                "\nAt line {line}, character {character}: replace {removed} characters with {:?}.",
                edit.replacement
            ));
        }
    }
    result
}

fn edits<'a>(old: &str, new: &'a str) -> Vec<Edit<'a>> {
    if let Some(breaks) = inserted_line_breaks(old, new) {
        return breaks;
    }
    // Matching line boundaries let us name separate edits without repeating
    // otherwise unchanged lines. One exact span covers changed boundaries.
    if old.bytes().filter(|byte| *byte == b'\n').count()
        == new.bytes().filter(|byte| *byte == b'\n').count()
    {
        let mut result = Vec::new();
        let mut offset = 0;
        for (before, after) in old.split('\n').zip(new.split('\n')) {
            if before != after {
                if result.len() == MAX_EDITS {
                    break;
                }
                result.push(edit(before, after, offset));
            }
            offset += before.len() + 1;
        }
        if result.len() < MAX_EDITS {
            return result;
        }
    }
    vec![edit(old, new, 0)]
}

fn inserted_line_breaks<'a>(old: &str, new: &'a str) -> Option<Vec<Edit<'a>>> {
    let mut old_chars = old.char_indices().peekable();
    let mut old_offset = 0;
    let mut break_start = None;
    let mut result = Vec::new();
    for (index, ch) in new.char_indices() {
        if old_chars
            .peek()
            .is_some_and(|(_, expected)| *expected == ch)
        {
            if let Some(start) = break_start.take() {
                result.push(Edit {
                    start: old_offset,
                    end: old_offset,
                    replacement: &new[start..index],
                });
                if result.len() > MAX_EDITS {
                    return None;
                }
            }
            let (start, matched) = old_chars.next().unwrap();
            old_offset = start + matched.len_utf8();
        } else if ch == '\n' {
            break_start.get_or_insert(index);
        } else {
            return None;
        }
    }
    if old_chars.next().is_some() {
        return None;
    }
    if let Some(start) = break_start {
        result.push(Edit {
            start: old_offset,
            end: old_offset,
            replacement: &new[start..],
        });
        if result.len() > MAX_EDITS {
            return None;
        }
    }
    Some(result)
}

fn edit<'a>(old: &str, new: &'a str, offset: usize) -> Edit<'a> {
    let (start, old_end, new_end) = replacement_span(old, new);
    Edit {
        start: offset + start,
        end: offset + old_end,
        replacement: &new[start..new_end],
    }
}

fn replacement_span(old: &str, new: &str) -> (usize, usize, usize) {
    let mut prefix = 0;
    for (a, b) in old.chars().zip(new.chars()) {
        if a != b {
            break;
        }
        prefix += a.len_utf8();
    }
    let mut old_end = old.len();
    let mut new_end = new.len();
    while old_end > prefix && new_end > prefix {
        let a = old[..old_end].chars().next_back().unwrap();
        let b = new[..new_end].chars().next_back().unwrap();
        if a != b {
            break;
        }
        old_end -= a.len_utf8();
        new_end -= b.len_utf8();
    }
    (prefix, old_end, new_end)
}

fn position(text: &str, offset: usize) -> (usize, usize) {
    let prefix = &text[..offset];
    (
        prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
        prefix.rsplit('\n').next().unwrap().chars().count() + 1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_reconstruct_validated_text_and_locate_unicode_insertions() {
        let many_old = (0..=MAX_EDITS)
            .map(|n| format!("line-{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let many_new = (0..=MAX_EDITS)
            .map(|n| format!("updated-{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        for (old, new) in [
            ("éx\nline-001", "é suffixx\nline-001 tail"),
            ("SameSame", "Same\n\nSame"),
            ("SameSameSame", "Same\n\nSame\n\nSame"),
            ("first\nsecond", "first suffix\nsecond"),
            ("old text", "new text"),
            ("deleted text", "deleted"),
            (&many_old, &many_new),
        ] {
            let mut rebuilt = old.to_owned();
            for edit in edits(old, new).into_iter().rev() {
                rebuilt.replace_range(edit.start..edit.end, edit.replacement);
            }
            assert_eq!(rebuilt, new);
        }
        let correction = note("éx\nline-001", "é suffixx\nline-001 tail");
        assert!(correction.contains("line 1, character 2: insert \" suffix\""));
        assert!(correction.contains("line 2, character 9: insert \" tail\""));
        assert!(!correction.contains("line-001"));
        assert!(note("SameSame", "Same\n\nSame").contains("insert a paragraph break"));
        assert_eq!(
            note("SameSameSame", "Same\n\nSame\n\nSame")
                .matches("insert a paragraph break")
                .count(),
            2
        );
        assert_eq!(edits(&many_old, &many_new).len(), 1);
    }
}
