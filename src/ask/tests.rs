use super::*;
use crate::json;

fn request(arguments: &str) -> Result<Request, String> {
    Request::parse("call", &json::parse(arguments).unwrap())
}

const TWO: &str = r#"{"questions":[
    {"id":"rows","title":"Rows","question":"Which rows?","options":[
        {"id":"visible","label":"Visible rows","recommended":true},
        {"id":"all","label":"All report rows"}]},
    {"id":"formats","title":"Formats","question":"Which formats?","multiple":true,"options":[
        {"id":"csv","label":"CSV"},{"id":"xlsx","label":"XLSX"}]}]}"#;

fn answer(id: &str, choices: &[&str], text: &str) -> Answer {
    Answer {
        id: id.into(),
        choices: choices.iter().map(|&choice| choice.into()).collect(),
        text: text.into(),
    }
}

#[test]
fn questions_are_unique_and_round_trip_as_arguments() {
    let parsed = request(TWO).unwrap();
    assert_eq!(parsed.subject(), "2 questions");
    assert!(parsed.questions[1].is_multiple());
    assert_eq!(Request::parse("call", &parsed.arguments()).unwrap(), parsed);
    let question = |id: usize| format!(r#"{{"id":"q{id}","title":"T","question":"Q"}}"#);
    assert!(request(r#"{"questions":[]}"#).is_err());
    assert!(request(&format!(r#"{{"questions":[{0},{0}]}}"#, question(1))).is_err());
    assert!(request(r#"{"questions":[{"title":"T","question":"Q"}]}"#).is_err());
    assert!(request(r#"{"questions":[{"id":"q","title":" ","question":"Q"}]}"#).is_err());
    let duplicate = r#"{"id":"o","label":"L"}"#;
    assert!(
        request(&format!(
            r#"{{"questions":[{{"id":"q","title":"T","question":"Q","options":[{duplicate},{duplicate}]}}]}}"#
        ))
        .is_err()
    );
}

#[test]
fn many_questions_options_and_long_texts_are_accepted() {
    // Past the former four questions, six options and per-field byte caps.
    let long = "word ".repeat(2000);
    let options = (0..20)
        .map(|id| {
            Value::object([
                (
                    "id",
                    Value::string(format!("option-{id}-{}", "x".repeat(100))),
                ),
                ("label", Value::string(format!("Label {id} {long}"))),
                ("description", Value::string(&long)),
            ])
        })
        .collect::<Vec<_>>();
    let questions = (0..12)
        .map(|id| {
            Value::object([
                ("id", Value::string(format!("q{id}"))),
                (
                    "title",
                    Value::string(format!("Title {id} {}", "t".repeat(300))),
                ),
                ("question", Value::string(&long)),
                ("context", Value::string(&long)),
                ("options", Value::Array(options.clone())),
            ])
        })
        .collect::<Vec<_>>();
    let arguments = Value::object([("questions", Value::Array(questions))]);
    let parsed = Request::parse("call", &arguments).unwrap();
    assert_eq!(parsed.questions.len(), 12);
    assert_eq!(parsed.questions[11].options.len(), 20);
    assert_eq!(Request::parse("call", &parsed.arguments()).unwrap(), parsed);
    let last = &parsed.questions[11];
    let chosen = answer("q11", &[&last.options[19].id], "");
    last.check_answer(&chosen).unwrap();
}

#[test]
fn one_question_and_its_draft_round_trip_in_the_current_format() {
    let single =
        request(r#"{"questions":[{"id":"format","title":"Format","question":"Which format?"}]}"#)
            .unwrap();
    assert_eq!(single.questions[0].id, "format");
    assert_eq!(single.subject(), "Format");
    let event = json::parse(
        r#"{"type":"user_question","tool_call_id":"call","questions":[{"id":"format","title":"Format","question":"Which format?"}]}"#,
    )
    .unwrap();
    assert_eq!(Request::from_event(&event).unwrap(), single);
    let draft = Draft::parse(
        &json::parse(r#"{"id":"call","step":0,"answers":[{"choices":[],"other":true,"text":"Keep dates","caret":4,"row":0}]}"#).unwrap(),
    )
    .unwrap();
    assert_eq!(draft.step, 0);
    assert!(draft.answers[0].other);
    assert_eq!(draft.answers[0].caret, 4);
    assert_eq!(Draft::parse(&draft.value()).unwrap(), draft);
    assert!(request(r#"{"title":"Format","question":"Which format?"}"#).is_err());
    assert!(Request::from_event(&json::parse(r#"{"tool_call_id":"call","question":{"title":"Format","question":"Which format?"}}"#).unwrap()).is_err());
    assert!(
        Draft::parse(
            &json::parse(r#"{"id":"call","text":"Keep dates","cursor":4,"choice":null}"#).unwrap()
        )
        .is_err()
    );
}

#[test]
fn every_question_needs_its_own_valid_answer_in_order() {
    let parsed = request(TWO).unwrap();
    let valid = vec![
        answer("rows", &[], "Only my team"),
        answer("formats", &["csv", "xlsx"], ""),
    ];
    parsed.check(&valid).unwrap();
    for invalid in [
        vec![valid[0].clone()],
        vec![valid[1].clone(), valid[0].clone()],
        vec![answer("rows", &["visible", "all"], ""), valid[1].clone()],
        vec![answer("rows", &["missing"], ""), valid[1].clone()],
        vec![answer("rows", &[], "  "), valid[1].clone()],
        vec![valid[0].clone(), answer("formats", &["csv", "csv"], "")],
    ] {
        assert!(parsed.check(&invalid).is_err());
    }
    let result = Reply::Answered(valid.clone()).result();
    assert_eq!(Reply::parse(&result), Some(Reply::Answered(valid)));
    assert!(
        Reply::parse(
            &json::parse(r#"{"outcome":"answered","choice":"visible","text":""}"#).unwrap()
        )
        .is_none()
    );
}
