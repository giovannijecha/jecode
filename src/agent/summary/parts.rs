use crate::{context::source, json::Value};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Cursor {
    pub record: usize,
    offset: usize,
}

/// Keeps records intact where possible; an oversized record continues at the
/// exact UTF-8 boundary in the next portion instead of losing its middle.
pub(super) fn portion(
    records: &[(usize, String)],
    messages: &[Value],
    start: Cursor,
    budget: usize,
) -> (String, Cursor) {
    let mut cursor = start;
    let mut text = String::new();
    while let Some((index, body)) = records.get(cursor.record) {
        let label = format!(
            "\nSource: {} (record byte offset {})\n",
            source::record(messages, *index).encode(),
            cursor.offset,
        );
        let available = budget.saturating_sub(text.len() + label.len());
        if available == 0 {
            break;
        }
        let remaining = &body[cursor.offset..];
        if remaining.len() > available && !text.is_empty() {
            break;
        }
        let take = remaining.floor_char_boundary(available.min(remaining.len()));
        if take == 0 && !remaining.is_empty() {
            break;
        }
        text.push_str(&label);
        text.push_str(&remaining[..take]);
        cursor.offset += take;
        if cursor.offset == body.len() {
            cursor.record += 1;
            cursor.offset = 0;
        } else {
            break;
        }
    }
    (text, cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_unicode_records_keep_every_byte_in_order() {
        let body = format!(
            "START {} MIDDLE {} END",
            "日本Ω".repeat(200),
            "data ".repeat(300)
        );
        let records = vec![(1, body.clone())];
        let mut cursor = Cursor::default();
        let mut restored = String::new();
        while cursor.record < records.len() {
            let (text, next) = portion(&records, &[], cursor, 320);
            assert!(next != cursor);
            assert!(text.len() <= 320);
            restored.push_str(text.split_once(")\n").unwrap().1);
            cursor = next;
        }
        assert_eq!(restored, body);
    }

    #[test]
    fn an_insufficient_budget_does_not_advance_or_drop_a_record() {
        let records = vec![(1, "Keep the original constraint".into())];
        let cursor = Cursor::default();
        let (text, next) = portion(&records, &[], cursor, 2);
        assert!(text.is_empty());
        assert!(next == cursor);
    }
}
