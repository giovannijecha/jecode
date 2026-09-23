//! Readable append-only corrections for terminal scrollback.

const MAX_PASSAGE_LINES: usize = 4;

pub(super) fn display(streamed: &str, final_text: &str) -> String {
    if let Some(passage) = passage(streamed, final_text) {
        format!("# Updated response\n\n{passage}")
    } else {
        format!("# Updated response (replaces earlier text)\n\n{final_text}")
    }
}

fn passage(streamed: &str, final_text: &str) -> Option<String> {
    // Outside fenced code, the owned Markdown renderer treats source lines
    // independently. Fences and uncertain boundaries use the complete answer.
    if streamed.contains("```")
        || final_text.contains("```")
        || streamed.contains('\r')
        || final_text.contains('\r')
    {
        return None;
    }
    let old: Vec<_> = streamed.split('\n').collect();
    let new: Vec<_> = final_text.split('\n').collect();
    let mut start = 0;
    while start < old.len() && start < new.len() && old[start] == new[start] {
        start += 1;
    }
    let (mut old_end, mut new_end) = (old.len(), new.len());
    while old_end > start && new_end > start && old[old_end - 1] == new[new_end - 1] {
        old_end -= 1;
        new_end -= 1;
    }
    if start == 0 && new_end == new.len() {
        return None;
    }
    let changed = &new[start..new_end];
    let previous = &old[start..old_end];
    if previous.len() > MAX_PASSAGE_LINES
        || changed
            .iter()
            .any(|line| !line.is_empty() && previous.contains(line))
    {
        return None;
    }
    let first = changed.iter().position(|line| !line.is_empty())?;
    let last = changed.iter().rposition(|line| !line.is_empty())? + 1;
    (last - first <= MAX_PASSAGE_LINES).then(|| changed[first..last].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_bounded_passages_and_complete_answers_when_boundaries_are_uncertain() {
        assert_eq!(
            display(
                "first\nline-000\nline-001",
                "first suffix\n\nline-000\nline-001"
            ),
            "# Updated response\n\nfirst suffix"
        );
        assert_eq!(
            display("SameSame", "Same\n\nSame"),
            "# Updated response (replaces earlier text)\n\nSame\n\nSame"
        );
        assert_eq!(
            display("A\n```rust\nold\n```", "A\n```rust\nnew\n```"),
            "# Updated response (replaces earlier text)\n\nA\n```rust\nnew\n```"
        );
        assert_eq!(
            display("intro\nUse `old`.\noutro", "intro\nUse `new`.\noutro"),
            "# Updated response\n\nUse `new`."
        );
        assert_eq!(
            display(
                "intro\nold one\nsteady\nold two\noutro",
                "intro\nnew one\nsteady\nnew two\noutro"
            ),
            "# Updated response (replaces earlier text)\n\nintro\nnew one\nsteady\nnew two\noutro"
        );
    }
}
