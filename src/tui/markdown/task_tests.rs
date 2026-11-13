//! Task list items render Ask's circles, keep their state colours, hang their
//! wrapped words under the title and keep each row's source position.
use super::*;

const SOURCE: &str = "- [ ] Pulire i file temporanei della build\n\
                      - [x] Ambito della pulizia confermato\n  \
                      - [X] Cache rimossa\n\
                      * [x] Star\n\
                      - plain [x] item";

#[test]
fn task_items_show_circles_and_hang_their_wrapped_words() {
    let rows = render(SOURCE, 20);
    let plain: Vec<String> = rows
        .iter()
        .map(|row| row.plain().trim_end().to_owned())
        .collect();
    assert_eq!(
        plain,
        [
            "○ Pulire i file",
            "  temporanei della",
            "  build",
            "◉ Ambito della",
            "  pulizia confermato",
            "  ◉ Cache rimossa",
            "◉ Star",
            "• plain [x] item",
        ]
    );
    // The mark alone carries the state colour; the words stay body.
    let mark = |row: usize| (rows[row].spans[0].text.as_str(), rows[row].spans[0].style);
    assert_eq!(mark(0), ("○ ", MUTED));
    assert_eq!(mark(3), ("◉ ", GOOD));
    assert_eq!(mark(5), ("  ◉ ", GOOD));
    assert_eq!(mark(6), ("◉ ", GOOD));
    assert!(rows[0].spans[1..].iter().all(|span| span.style == BODY));
    // Literal brackets inside text, and the source itself, stay as written.
    assert!(SOURCE.contains("- [ ] Pulire") && SOURCE.contains("[X] Cache"));
}

#[test]
fn wrapped_task_rows_keep_their_source_positions_and_fit_every_width() {
    for columns in [8, 12, 20, 40, 80] {
        let rows = render(SOURCE, columns);
        assert!(rows.iter().all(|row| text::cells(&row.plain()) <= columns));
        assert!(!rows.iter().any(|row| row.plain().contains(['☐', '☑'])));
        // Each item's rows start at its source line, in order; continuations
        // advance through the same line.
        let mut item = 0;
        let starts: Vec<usize> = std::iter::once(0)
            .chain(SOURCE.match_indices('\n').map(|(index, _)| index + 1))
            .collect();
        let mut previous = None;
        for row in &rows {
            let origin = row.origin.unwrap();
            let plain = row.plain();
            let opens = plain.trim_start().starts_with(['○', '◉', '•']);
            if opens {
                assert_eq!(origin.line, starts[item], "{columns}: {plain:?}");
                item += 1;
            } else {
                let (line, character) = previous.unwrap();
                assert_eq!(origin.line, line, "{columns}: {plain:?}");
                assert!(origin.character > character, "{columns}: {plain:?}");
            }
            previous = Some((origin.line, origin.character));
        }
        assert_eq!(item, starts.len());
    }
}
