use super::*;
use crate::terminal::lab::glyph::UNICODE;

fn choices(labels: &[&str]) -> Vec<Choice> {
    labels
        .iter()
        .map(|label| Choice {
            label: (*label).into(),
            detail: String::new(),
        })
        .collect()
}

/// Long enough to be filterable: three models plus six fillers `x1..x6`.
fn picker(query: &str, selected: usize) -> Picker {
    let mut choices = vec![
        Choice {
            label: "opus-5.5".into(),
            detail: "most capable".into(),
        },
        Choice {
            label: "sonnet-5".into(),
            detail: "balanced".into(),
        },
        Choice {
            label: "haiku-4.5".into(),
            detail: "fast".into(),
        },
    ];
    choices.extend(choices_of(6));
    Picker {
        title: "Model".into(),
        choices,
        query: query.into(),
        selected,
    }
}

fn choices_of(count: usize) -> Vec<Choice> {
    (1..=count)
        .map(|n| Choice {
            label: format!("x{n}"),
            detail: String::new(),
        })
        .collect()
}

#[test]
fn short_lists_are_numbered_menus() {
    let menu = Picker {
        title: "Effort".into(),
        choices: choices(&["low", "high"]),
        query: String::new(),
        selected: 1,
    };
    let rows = rows(&menu, 30, &UNICODE);
    let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(texts[1], " Effort");
    assert_eq!(texts[2], "   1  low");
    assert!(texts[3].starts_with(" › 2  high"));
    assert!(rows[2].spans.contains(&(3..4, Tone::Accent)));
    assert!(texts[5].starts_with(" ↑↓ move · 1-2"));
    assert!(text::width(texts[5]) <= 30);
    assert_eq!(
        (digit(&menu, '2'), digit(&menu, '3'), digit(&menu, '0')),
        (Some(1), None, None)
    );
    assert_eq!(digit(&picker("", 0), '1'), None);
}

#[test]
fn substring_beats_subsequence_and_ties_keep_order() {
    let list = choices(&["gpt-oss", "opus-5.5", "o-p-s", "Opus-4.7"]);
    let found = matches(&list, "OP");
    let order: Vec<usize> = found.iter().map(|found| found.index).collect();
    assert_eq!(order, [1, 3, 2]);
    assert_eq!((found[0].hits.len(), &found[0].hits[0]), (1, &(0..2)));
    assert_eq!(found[2].hits, [0..1, 2..3]);
}

#[test]
fn empty_query_keeps_everything_and_misses_drop_out() {
    let list = choices(&["a", "b"]);
    assert_eq!(matches(&list, "").len(), 2);
    assert!(matches(&list, "z").is_empty());
}

#[test]
fn non_ascii_hits_are_byte_ranges_on_char_boundaries() {
    let list = choices(&["Perché"]);
    let found = matches(&list, "hé");
    assert_eq!((found[0].hits.len(), &found[0].hits[0]), (1, &(4..7)));
}

#[test]
fn layout_counts_highlights_and_selects() {
    let rows = rows(&picker("s", 1), 40, &UNICODE);
    let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
    assert!(texts[1].starts_with(" Model › s"));
    assert!(texts[1].ends_with("2 of 9"));
    assert_eq!(text::width(texts[1]), 40);
    assert_eq!(texts[2], "   sonnet-5   balanced");
    assert!(texts[3].starts_with(" › opus-5.5   most capable"));
    assert_eq!(rows[3].tone, Tone::User);
    assert_eq!(text::width(texts[3]), 40);
    assert!(rows[2].spans.contains(&(3..4, Tone::Accent)));
    assert_eq!(texts[5], " ↑↓ move · Enter select · Esc close");
}

#[test]
fn no_matches_and_overflow_are_spelled_out() {
    let none = rows(&picker("zz", 0), 40, &UNICODE);
    assert_eq!(none[2].text, "   No matches");
    let labels: Vec<String> = (0..12).map(|n| format!("model-{n}")).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let long = Picker {
        choices: choices(&labels),
        ..picker("", 0)
    };
    let rows = rows(&long, 40, &UNICODE);
    assert_eq!(rows.len(), 2 + VISIBLE + 1 + 2);
    assert_eq!(rows[2 + VISIBLE].text, "   … 4 more");
}

#[test]
fn long_labels_and_queries_fit_twenty_columns_without_losing_selection() {
    let mut open = picker("very-long-filter-term", 6);
    open.title = "Reasoning effort for account".into();
    open.choices[6].label = "gpt-5.6-long-model-name-with-emoji-🧪".into();
    let filtered = rows(&open, 20, &UNICODE);
    assert!(
        filtered.iter().all(|row| text::width(&row.text) <= 20),
        "{filtered:?}"
    );

    let menu = Picker {
        title: "Reasoning effort for account".into(),
        choices: vec![Choice {
            label: "long-choice-that-must-clip".into(),
            detail: "long-description".into(),
        }],
        query: String::new(),
        selected: 0,
    };
    let rows = rows(&menu, 20, &UNICODE);
    assert!(
        rows.iter().all(|row| text::width(&row.text) <= 20),
        "{rows:?}"
    );
    assert!(rows.iter().any(|row| row.tone == Tone::User));
}
