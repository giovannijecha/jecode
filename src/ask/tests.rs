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
fn questions_are_bounded_unique_and_round_trip_as_arguments() {
    let parsed = request(TWO).unwrap();
    assert_eq!(parsed.subject(), "2 questions");
    assert!(parsed.questions[1].is_multiple());
    assert_eq!(Request::parse("call", &parsed.arguments()).unwrap(), parsed);
    let question = |id: usize| format!(r#"{{"id":"q{id}","title":"T","question":"Q"}}"#);
    let many = (0..=MAX_QUESTIONS).map(question).collect::<Vec<_>>();
    assert!(request(&format!(r#"{{"questions":[{}]}}"#, many.join(","))).is_err());
    assert!(request(r#"{"questions":[]}"#).is_err());
    assert!(request(&format!(r#"{{"questions":[{0},{0}]}}"#, question(1))).is_err());
    let options = (0..=MAX_OPTIONS)
        .map(|id| format!(r#"{{"id":"o{id}","label":"L"}}"#))
        .collect::<Vec<_>>();
    assert!(
        request(&format!(
            r#"{{"questions":[{{"id":"q","title":"T","question":"Q","options":[{}]}}]}}"#,
            options.join(",")
        ))
        .is_err()
    );
    assert!(request(r#"{"questions":[{"title":"T","question":"Q"}]}"#).is_err());
}

#[test]
fn an_earlier_single_question_loads_as_one_question() {
    let legacy = request(r#"{"title":"Format","question":"Which format?"}"#).unwrap();
    assert_eq!(legacy.questions[0].id, LEGACY_ID);
    assert_eq!(legacy.subject(), "Format");
    let event = json::parse(
        r#"{"type":"user_question","tool_call_id":"call","question":{"title":"Format","question":"Which format?"}}"#,
    )
    .unwrap();
    assert_eq!(Request::from_event(&event).unwrap(), legacy);
    let draft = Draft::parse(
        &json::parse(r#"{"id":"call","text":"Keep dates","cursor":4,"choice":null}"#).unwrap(),
    )
    .unwrap();
    assert_eq!(draft.step, 0);
    assert!(draft.answers[0].other);
    assert_eq!(draft.answers[0].caret, 4);
    assert_eq!(Draft::parse(&draft.value()).unwrap(), draft);
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
}
