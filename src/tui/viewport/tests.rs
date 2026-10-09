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
    view.scroll(Scroll::Rows(1));
    let before = view.view(&wide, &[], 1);
    let origin = before.lines[0].origin.unwrap();
    assert!(origin.character > 0 && before.lines[0].plain().contains("delta"));
    let narrow = vec![markdown::render(source, 18)];
    let after = view.view(&narrow, &[], 1);
    assert_eq!(after.lines[0].origin, Some(origin));
    assert!(after.lines[0].plain().contains("delta"));
}
