use super::{
    Key,
    editor::boundaries,
    model::{Block, Model},
    render::Renderer,
    style::{Row, Tone},
    view::{self, Layout},
};
use std::{ops::Range, time::Instant};

fn cursor_row(model: &Model, columns: usize) -> (Row, Range<usize>) {
    let mut rows = view::chrome(model, columns, 12)
        .into_iter()
        .filter_map(|row| {
            row.spans
                .iter()
                .find(|(_, tone)| *tone == Tone::Cursor)
                .map(|(range, _)| (row.clone(), range.clone()))
        });
    let found = rows.next().expect("one visible cursor");
    assert!(rows.next().is_none());
    found
}

#[test]
fn block_cursor_highlights_ciao_without_moving_any_letter_or_overwriting_text() {
    let mut model = Model::new(Instant::now());
    assert!(model.editor.insert("ciao"));
    for (position, marked) in [(0, "c"), (1, "i"), (2, "a"), (3, "o"), (4, " ")] {
        model.editor.cursor = position;
        let (row, span) = cursor_row(&model, 80);
        assert_eq!(
            row.text,
            if position == 4 {
                "› ciao "
            } else {
                "› ciao"
            }
        );
        assert_eq!(&row.text[span], marked);
        let plain = row.paint(false);
        assert_eq!(plain.matches("\x1b[7m").count(), 1);
        assert_eq!(
            plain.replace("\x1b[7m", "").replace("\x1b[27m", ""),
            row.text
        );
        assert!(!plain.contains('|'));

        let mut edited = Model::new(Instant::now());
        edited.editor.insert("ciao");
        edited.editor.cursor = position;
        edited.input(Key::Text("X".into()), Instant::now());
        assert_eq!(
            edited.editor.text,
            format!("{}X{}", &"ciao"[..position], &"ciao"[position..])
        );
        edited.input(Key::Backspace, Instant::now());
        assert_eq!(edited.editor.text, "ciao");
        if position < 4 {
            edited.input(Key::Delete, Instant::now());
            assert_eq!(
                edited.editor.text,
                format!("{}{}", &"ciao"[..position], &"ciao"[position + 1..])
            );
        }
    }
}

#[test]
fn empty_lines_tabs_unicode_and_wraps_keep_cursor_on_the_displayed_unit() {
    let mut model = Model::new(Instant::now());
    let (row, span) = cursor_row(&model, 80);
    assert_eq!(&row.text[span], " ");
    assert!(row.text.contains("Ask anything…"));
    assert!(row.paint(false).contains("Ask anything…"));

    model.editor.insert("ab\ncd\n");
    for (position, marked) in [(2, " "), (3, "c"), (5, " "), (6, " ")] {
        model.editor.cursor = position;
        let (row, span) = cursor_row(&model, 80);
        assert_eq!(&row.text[span], marked);
    }

    model.editor.replace("a\tb");
    model.editor.cursor = 1;
    let (row, span) = cursor_row(&model, 80);
    assert_eq!(row.text, "› a   b");
    assert_eq!(&row.text[span], " ");

    model.editor.replace("e\u{301} 中 👩\u{200d}💻 각");
    for pair in boundaries(&model.editor.text).windows(2) {
        let unit = &model.editor.text[pair[0]..pair[1]];
        if ["e\u{301}", "中", "👩\u{200d}💻", "각"].contains(&unit) {
            model.editor.cursor = pair[0];
            let (row, span) = cursor_row(&model, 80);
            assert_eq!(&row.text[span], unit);
            assert!(row.paint(false).contains(unit));
        }
    }

    model.editor.replace("abcdefghijklmnopqrstuVW");
    for position in [20, 21, 22] {
        model.editor.cursor = position;
        let rows = view::chrome(&model, 25, 12);
        assert!(
            rows.iter().any(|row| row.text == "› abcdefghijklmnopqrstu"),
            "{rows:?}"
        );
        assert!(rows.iter().any(|row| row.text == "  VW"));
        let (row, span) = cursor_row(&model, 25);
        assert_eq!(&row.text[span], &model.editor.text[position..position + 1]);
    }
}

#[test]
fn cursor_stays_visible_without_duplicate_chrome_or_transcript_during_resize() {
    let mut model = Model::new(Instant::now());
    model.blocks.push(Block {
        speaker: "You",
        text: "immutable transcript marker".into(),
    });
    model
        .editor
        .insert(&"long draft with tabs\tand text\n".repeat(10));
    model.editor.cursor = model.editor.text.len() / 2;
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    renderer.draw(layout.frame(&model, 80, 12), (80, 12), false);
    for columns in [40, 25, 80, 32, 80] {
        let rows = layout.frame(&model, columns, 12);
        assert_eq!(
            rows.iter()
                .flat_map(|row| &row.spans)
                .filter(|(_, tone)| *tone == Tone::Cursor)
                .count(),
            1
        );
        assert_eq!(
            rows.iter().filter(|row| row.text.starts_with('─')).count(),
            2
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.text.contains("Local demo"))
                .count(),
            1
        );
        assert!(
            !renderer
                .draw(rows, (columns, 12), false)
                .contains("immutable transcript marker")
        );
    }
}
