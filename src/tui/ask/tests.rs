use super::Panel;
use crate::{
    ask::{AnswerDraft, Draft, Request},
    json,
    tui::{
        line::Line,
        state::State,
        theme::{BADGE_IDLE, SELECTED_SURFACE},
        view,
    },
};

fn request(arguments: &str) -> Request {
    Request::parse("call", &json::parse(arguments).unwrap()).unwrap()
}

const SINGLE: &str = r#"{"questions":[{"id":"rows","title":"Rows",
    "question":"Which rows should the CSV include?",
    "context":"The report currently has active filters.","options":[
    {"id":"visible","label":"Visible rows","description":"Keep filters","recommended":true},
    {"id":"all","label":"All rows","description":"Ignore filters"}]}]}"#;

const SEVERAL: &str = r#"{"questions":[
    {"id":"formats","title":"Formats","question":"Which formats?","multiple":true,
     "options":[{"id":"csv","label":"CSV"},{"id":"xlsx","label":"XLSX"}]},
    {"id":"delivery","title":"Delivery","question":"Where should exports go?"}]}"#;

fn dock(request: &Request, panel: &Panel, width: usize, height: usize) -> Vec<Line> {
    let state = State {
        width,
        height,
        question: Some(request.clone()),
        ask_panel: Some(panel.clone()),
        ..State::default()
    };
    view::composer_frame(&state, "fixture", "folder").live
}

fn plain(rows: &[Line]) -> Vec<String> {
    rows.iter()
        .map(|row| row.plain().trim_end().to_owned())
        .collect()
}

#[test]
fn the_cursor_moves_freely_and_only_space_enter_or_typing_select() {
    let request = request(SINGLE);
    let question = &request.questions[0];
    let mut panel = Panel::new(&request, None);
    assert!(panel.move_row(question, false));
    assert!(panel.entry().picked.is_empty());
    assert!(panel.select(question));
    assert_eq!(panel.entry().picked, [1]);
    assert!(panel.type_text(question, "Only my team"));
    let entry = panel.entry();
    assert_eq!((entry.row, entry.other, entry.picked.len()), (2, true, 0));
    assert!(!panel.select(question), "Space is typed in the own words");
    assert!(panel.move_row(question, true));
    assert!(panel.choose(question));
    assert_eq!(
        (panel.entry().picked.clone(), panel.entry().other),
        (vec![1], false)
    );
    panel.entries[0].row = 2;
    panel.entries[0].editor.replace(String::new());
    assert!(!panel.choose(question), "empty own words are no answer");

    let several = self::request(SEVERAL);
    let mut panel = Panel::new(&several, None);
    let formats = &several.questions[0];
    panel.select(formats);
    panel.move_row(formats, false);
    panel.select(formats);
    panel.select(formats);
    assert_eq!(panel.entry().picked, [0]);
    assert_eq!(panel.answers(&several), Err(1));
    panel.switch(&several, true);
    panel.type_text(&several.questions[1], "Email");
    let answers = panel.answers(&several).unwrap();
    assert_eq!(answers[0].choices, ["csv"]);
    assert_eq!(answers[1].text, "Email");
    let restored = Panel::new(&several, Some(&panel.draft(&several)));
    assert_eq!(restored, panel);
}

#[test]
fn an_earlier_draft_restores_its_choice_and_words() {
    let request = request(SINGLE);
    let draft = Draft {
        id: "call".into(),
        step: 0,
        answers: vec![AnswerDraft {
            choices: vec!["all".into(), "missing".into()],
            other: false,
            text: "Keep dates".into(),
            caret: 4,
            row: 9,
        }],
    };
    let panel = Panel::new(&request, Some(&draft));
    let entry = panel.entry();
    assert_eq!((entry.picked.clone(), entry.row), (vec![1], 2));
    assert_eq!(
        (entry.editor.text.as_str(), entry.editor.cursor),
        ("Keep dates", 4)
    );
    let other = Draft {
        id: "elsewhere".into(),
        ..draft
    };
    assert_eq!(
        Panel::new(&request, Some(&other)),
        Panel::new(&request, None)
    );
}

#[test]
fn the_question_takes_the_composer_place_with_marks_facts_and_chip() {
    let request = request(SINGLE);
    let panel = Panel::new(&request, None);
    let rows = dock(&request, &panel, 60, 30);
    let chip = format!(
        "› ○ Visible rows  Keep filters{} recommended",
        " ".repeat(15)
    );
    assert_eq!(
        plain(&rows),
        [
            " ?  Waiting for you",
            &"─".repeat(60),
            "  Which rows should the CSV include?",
            "  The report currently has active filters.",
            "",
            &chip,
            "  ○ All rows      Ignore filters",
            "  ○ Other         Type your own answer",
            &"─".repeat(60),
            "   Enter  choose    ↑↓  move    Esc  back to chat",
        ]
    );
    assert_eq!(
        rows[5].plain().chars().count(),
        60,
        "the chip ends at edge − 2"
    );
    for span in &rows[5].spans {
        let expected = if span.text == " recommended " {
            BADGE_IDLE
        } else {
            SELECTED_SURFACE
        };
        assert_eq!(span.background, Some(expected), "{:?}", span.text);
    }
    assert!(
        rows[6]
            .spans
            .iter()
            .all(|span| span.background != Some(SELECTED_SURFACE))
    );
}

#[test]
fn own_words_wrap_under_the_fact_column_with_the_caret_at_the_end() {
    let request = request(SINGLE);
    let mut panel = Panel::new(&request, None);
    panel.type_text(
        &request.questions[0],
        "Only the rows of the selected team, and archived reports",
    );
    let state = State {
        width: 40,
        height: 30,
        question: Some(request.clone()),
        ask_panel: Some(panel),
        ..State::default()
    };
    let frame = view::composer_frame(&state, "fixture", "folder");
    let rows = plain(&frame.live);
    let own = rows
        .iter()
        .position(|row| row.starts_with("› ◉ Other"))
        .unwrap();
    assert_eq!(rows[own], "› ◉ Other         Only the rows of the");
    assert!(rows[own + 1].starts_with(&" ".repeat(18)));
    let (row, column) = frame.cursor.unwrap();
    assert!(row > own && column > 18 && column < 38, "{row} {column}");
    assert!(rows.last().unwrap().contains("Enter  send"));
}

#[test]
fn several_questions_show_a_step_row_and_free_text_input() {
    let request = request(SEVERAL);
    let mut panel = Panel::new(&request, None);
    panel.select(&request.questions[0]);
    panel.switch(&request, true);
    let rows = plain(&dock(&request, &panel, 60, 30));
    assert_eq!(
        rows[2],
        format!("  ☑ Formats   Delivery{}2 of 2", " ".repeat(30))
    );
    assert_eq!(rows[5], "› Type your answer");
    assert_eq!(
        rows.last().unwrap(),
        "   Enter  send    Ctrl+J  new line    ←→  questions",
        "keys that do not fit are left out"
    );
}

#[test]
fn a_question_taller_than_its_room_scrolls_with_the_cursor() {
    let mut arguments = json::parse(SINGLE).unwrap().encode();
    arguments = arguments.replace(
        "The report currently has active filters.",
        &"Filters decide which rows appear. ".repeat(12),
    );
    let request = Request::parse("call", &json::parse(&arguments).unwrap()).unwrap();
    let mut panel = Panel::new(&request, None);
    panel.move_row(&request.questions[0], false);
    let rows = dock(&request, &panel, 40, 18);
    let shown = plain(&rows);
    assert!(
        shown.iter().any(|row| row.starts_with("› ○ All rows")),
        "{shown:#?}"
    );
    assert!(shown.iter().any(|row| row.ends_with('┃')));
    assert!(shown.iter().any(|row| row.ends_with('│')));
    assert!(rows.len() <= 12);
}
