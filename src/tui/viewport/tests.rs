use super::*;
use crate::tui::{markdown, theme::BODY};

fn blocks(count: usize) -> Vec<Vec<Line>> {
    (0..count)
        .map(|i| vec![Line::new(&format!("line {i}"), BODY).at_source(0)])
        .collect()
}

#[test]
fn reading_does_not_follow_new_output_and_paging_to_the_bottom_resumes_following() {
    let mut view = Viewport::default();
    let mut source = blocks(30);
    assert_eq!(view.view(&source, &[], 10).lines[0].plain(), "line 20");
    view.scroll(Scroll::Page(true));
    assert_eq!(view.view(&source, &[], 10).lines[0].plain(), "line 11");
    source.extend(blocks(10));
    let read = view.view(&source, &[], 10);
    assert_eq!(read.lines[0].plain(), "line 11");
    assert!(read.reading);
    view.scroll(Scroll::Rows(100));
    assert!(!view.view(&source, &[], 10).reading);
    source.push(vec![Line::new("latest", BODY)]);
    assert_eq!(
        view.view(&source, &[], 10).lines.last().unwrap().plain(),
        "latest"
    );
}

#[test]
fn reflow_keeps_the_same_source_line_inside_a_long_message() {
    let source = (0..100)
        .map(|i| format!("Paragraph {i:03}: one two three four five six seven eight nine ten.\n"))
        .collect::<String>();
    let mut view = Viewport::default();
    let wide = vec![markdown::render(&source, 79)];
    view.view(&wide, &[], 10);
    view.scroll(Scroll::Page(true));
    let before = view.view(&wide, &[], 10);
    let origin = before.lines[0].origin.unwrap();
    let narrow = vec![markdown::render(&source, 29)];
    let after = view.view(&narrow, &[], 10);
    assert_eq!(after.lines[0].origin.unwrap().line, origin.line);
    assert_eq!(view.view(&wide, &[], 10).lines[0], before.lines[0]);
}

#[test]
fn wheel_rows_empty_blocks_and_short_conversations_do_not_mutate_source() {
    let mut view = Viewport::default();
    let mut source = blocks(30);
    source.insert(3, vec![]);
    let original = source.clone();
    view.view(&source, &[], 10);
    view.scroll(Scroll::Rows(-3));
    assert_eq!(view.view(&source, &[], 10).lines[0].plain(), "line 17");
    view.scroll(Scroll::Start);
    assert_eq!(view.view(&source, &[], 10).lines[0].plain(), "line 0");
    view.scroll(Scroll::End);
    assert!(!view.view(&source, &[], 10).reading);
    assert_eq!(source, original);
    assert!(view.view(&[], &[], 10).lines.is_empty());
    assert_eq!(view.view(&blocks(1), &[], 10).lines.len(), 1);
}

#[test]
fn reaching_the_actual_bottom_resumes_following_even_after_start_or_reflow() {
    let mut view = Viewport::default();
    let mut short = blocks(3);
    view.scroll(Scroll::Start);
    assert!(!view.view(&short, &[], 10).reading);
    assert!(view.following());
    short.extend((3..15).map(|index| vec![Line::new(&format!("line {index}"), BODY)]));
    assert_eq!(view.view(&short, &[], 10).lines[0].plain(), "line 5");

    view.scroll(Scroll::Start);
    assert!(view.view(&short, &[], 10).reading);
    let fitting = blocks(8);
    assert!(!view.view(&fitting, &[], 10).reading);
    assert!(view.following());

    view.scroll(Scroll::Page(true));
    assert!(!view.view(&fitting, &[], 10).reading);
}

#[test]
fn reflow_of_a_wrapped_table_header_keeps_its_text_instead_of_the_separator() {
    let source =
        "| alpha bravo charlie delta echo foxtrot golf | State |\n|---|---|\n| value | ready |";
    let mut view = Viewport::default();
    let wide = vec![markdown::render(source, 30)];
    view.scroll(Scroll::Start);
    view.view(&wide, &[], 1);
    // Row 0 is the top border, row 1 the header's first wrapped row.
    view.scroll(Scroll::Rows(2));
    let before = view.view(&wide, &[], 1);
    let origin = before.lines[0].origin.unwrap();
    assert!(origin.character > 0 && before.lines[0].plain().starts_with("│ charlie delta"));
    let narrow = vec![markdown::render(source, 18)];
    let after = view.view(&narrow, &[], 1);
    assert_eq!(after.lines[0].origin, Some(origin));
    assert!(after.lines[0].plain().starts_with("│ charli"));
}

#[test]
fn a_folding_source_can_remap_its_reading_anchor_to_the_chapter_header() {
    let mut expanded = vec![
        vec![Line::new("chapter", BODY).at_source(0)],
        vec![
            Line::new("later call", BODY).at_source(0),
            Line::new("later output", BODY).at_source(1),
        ],
    ];
    expanded.extend(blocks(10));
    let mut view = Viewport::default();
    view.view(&expanded, &[], 4);
    view.scroll(Scroll::Start);
    view.view(&expanded, &[], 4);
    view.scroll(Scroll::Rows(2));
    assert_eq!(
        view.view(&expanded, &[], 4).lines[0].plain(),
        "later output"
    );

    view.remap_anchor(|block, origin| {
        assert_eq!(block, 1);
        assert_eq!(origin.unwrap().line, 1);
        Some(0)
    });
    let mut compact = vec![vec![Line::new("chapter", BODY).at_source(0)], vec![]];
    compact.extend(blocks(10));
    assert_eq!(view.view(&compact, &[], 4).lines[0].plain(), "chapter");
}

#[test]
fn remap_leaves_tail_following_and_unselected_reading_anchors_alone() {
    let source = blocks(20);
    let mut following = Viewport::default();
    following.view(&source, &[], 5);
    following.remap_anchor(|_, _| panic!("tail following must not be remapped"));
    assert!(following.following());
    assert_eq!(following.view(&source, &[], 5).lines[0].plain(), "line 15");

    let mut reading = Viewport::default();
    reading.scroll(Scroll::Start);
    reading.view(&source, &[], 5);
    reading.scroll(Scroll::Rows(3));
    assert_eq!(reading.view(&source, &[], 5).lines[0].plain(), "line 3");
    reading.remap_anchor(|block, _| {
        assert_eq!(block, 3);
        None
    });
    assert_eq!(reading.view(&source, &[], 5).lines[0].plain(), "line 3");
}
