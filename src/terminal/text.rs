//! Conservative display units. This is not a complete Unicode width database.
//! Long non-ASCII runs remain together in transcript wrapping.
pub fn boundaries(text: &str) -> Vec<usize> {
    let mut result = vec![0];
    let mut previous = '\0';
    for (index, current) in text.char_indices() {
        if index != 0
            && (previous.is_ascii() && current.is_ascii()
                || matches!(previous, '\n' | '\t')
                || matches!(current, '\n' | '\t'))
        {
            result.push(index);
        }
        previous = current;
    }
    if !text.is_empty() {
        result.push(text.len());
    }
    result
}

pub fn safe(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\n' | '\r' => '\n',
            '\t' => ' ',
            ch if ch.is_control()
                || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') =>
            {
                '?'
            }
            ch => ch,
        })
        .collect()
}

/// One cell for ASCII, Latin letters and our UI symbols; a conservative two-cell
/// fallback elsewhere. This is not a complete terminal Unicode width table.
pub fn width(text: &str) -> usize {
    text.chars()
        .map(|c| {
            if c.is_ascii()
                || matches!(c, '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{24f}' | '─' | '›' | '…' | '·' | '✓' | '\u{2800}'..='\u{28ff}')
            {
                1
            } else {
                2
            }
        })
        .sum()
}

pub fn wrap(text: &str, columns: usize) -> Vec<String> {
    let columns = columns.max(1);
    let clean = safe(text);
    let mut rows = Vec::new();
    for line in clean.split('\n') {
        let mut row = String::new();
        let mut used = 0;
        for pair in boundaries(line).windows(2) {
            let unit = &line[pair[0]..pair[1]];
            let size = width(unit);
            if used + size > columns && !row.is_empty() {
                let seam = row.rfind(' ').map(|index| index + 1).filter(|index| {
                    !unit.starts_with(' ')
                        && !row[..*index].trim().is_empty()
                        && boundaries(&row).contains(index)
                        && width(&row[*index..]) + size <= columns
                });
                if let Some(index) = seam {
                    let tail = row.split_off(index);
                    rows.push(std::mem::replace(&mut row, tail));
                    used = width(&row);
                } else {
                    rows.push(std::mem::take(&mut row));
                    used = 0;
                }
            }
            if size > columns {
                // Never split an unknown Unicode sequence to force it into a row.
                row.push_str(&".".repeat(columns.min(3)));
                used += columns.min(3);
            } else {
                row.push_str(unit);
                used += size;
            }
        }
        rows.push(row);
    }
    rows
}
