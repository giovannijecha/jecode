use super::*;
use crate::tui::markdown;

fn rendered(source: &str, columns: usize) -> Vec<String> {
    markdown::render(source, columns)
        .iter()
        .map(Line::plain)
        .collect()
}

#[test]
fn clean_columns_use_visual_widths_and_preserve_inline_styles() {
    let source = "| Name | State |\n| --- | --- |\n| **café** | 🟢 ready |\n| 界 | *done* |";
    let rows = markdown::render(source, 60);
    let plain: Vec<_> = rows.iter().map(Line::plain).collect();
    assert_eq!(
        plain,
        [
            "Name  State",
            "────  ────────",
            "café  🟢 ready",
            "界    done"
        ]
    );
    assert!(
        rows[2]
            .spans
            .iter()
            .any(|span| span.text == "café" && span.style == EMPHASIS)
    );
    assert!(
        rows[3]
            .spans
            .iter()
            .any(|span| span.text == "done" && span.style.contains('3'))
    );
    assert!(source.contains("**café**"));
}

#[test]
fn alignment_escape_empty_cells_and_surplus_cells_are_visible() {
    let source = "| L | C | R |\n| :--- | :---: | ---: |\n| a | b | 7 |\n| \\| | `x\\|y` | 12 |\n| missing |\n| extra | b | c | preserved |";
    let rows = rendered(source, 80);
    assert!(rows[2].ends_with('7'));
    assert!(rows[3].contains("x|y"));
    assert!(rows[3].contains('|'));
    assert!(rows[4].starts_with("missing"));
    assert!(rows[5].ends_with("c | preserved"));
    assert!(rows.iter().all(|row| text::cells(row) <= 80));
}

#[test]
fn narrow_tables_stack_labels_and_never_drop_cell_content() {
    let source = "| Name | State | Note |\n|---|---|---|\n| 界👩‍💻 | ready | a longer cell |";
    for width in [2, 6, 12, 24, 40] {
        let rows = rendered(source, width);
        assert!(rows.iter().all(|row| text::cells(row) <= width));
        let combined = rows.join("");
        assert!(combined.contains("界👩‍💻"));
        assert!(combined.contains("ready"));
        assert!(combined.replace(' ', "").contains("alongercell"));
    }
    let rows = rendered(source, 12);
    assert!(rows[0].starts_with("Name:"));
    assert!(rows.iter().any(|row| row.starts_with("State:")));
}

#[test]
fn tables_stop_at_blocks_and_fences_keep_literal_table_syntax() {
    let table = "| A | B |\n|---|---|\n| one | two |";
    for next in ["# Next", "> quote", "- list", "```text\ncode\n```", "---"] {
        let rows = rendered(&format!("{table}\n{next}"), 60);
        assert!(rows[2].contains("one"));
        assert!(rows.len() > 3);
    }
    let rows = rendered(&format!("```\n{table}\n```"), 60);
    assert_eq!(rows[1], "| A | B |");
    assert_eq!(rows[2], "|---|---|");
    assert_eq!(rendered("a | b\n---\ntext", 60)[0], "a | b");
    assert_eq!(rendered("|a|b|\n|---|bad|", 60)[1], "|---|bad|");
}

#[test]
fn long_tables_yield_during_measurement_and_wrapped_output() {
    let source = format!(
        "| A | B |\n|---|---|\n{}",
        (0..600)
            .map(|i| format!("| item{i} | body{i} |\n"))
            .collect::<String>()
    );
    let mut rows = markdown::Rows::new(&source, 40);
    for _ in 0..100 {
        assert!(rows.next().unwrap().is_empty());
    }
    let output: Vec<_> = rows.flatten().map(|row| row.plain()).collect();
    assert_eq!(output.len(), 602);
    assert!(output.last().unwrap().contains("body599"));
    let tall = format!("| A | B |\n|---|---|\n| {} | z |", "value ".repeat(2000));
    let steps: Vec<_> = markdown::Rows::new(&tall, 16).collect();
    assert!(steps.iter().all(|rows| rows.len() <= 32));
    assert!(steps.len() > 20);
}
