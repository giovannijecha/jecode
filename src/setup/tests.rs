use super::*;
use crate::json::Value;
use crate::test_support::{Directory, HttpFixture};
use std::collections::VecDeque;
use std::fs;

struct Prompts {
    inputs: VecDeque<String>,
    secrets: VecDeque<String>,
    messages: String,
}

impl Prompts {
    fn new(inputs: &[&str], secrets: &[&str]) -> Self {
        Self {
            inputs: inputs.iter().map(|text| text.to_string()).collect(),
            secrets: secrets.iter().map(|text| text.to_string()).collect(),
            messages: String::new(),
        }
    }
}

impl Prompter for Prompts {
    fn message(&mut self, text: &str) -> Result<(), String> {
        self.messages.push_str(text);
        self.messages.push('\n');
        Ok(())
    }
    fn input(&mut self, _: &str) -> Result<Option<String>, String> {
        Ok(self.inputs.pop_front())
    }
    fn secret(&mut self, _: &str) -> Result<Option<String>, String> {
        Ok(self.secrets.pop_front())
    }
}

fn key_response() -> Value {
    Value::object([("data", Value::object([]))])
}

fn catalog() -> Value {
    Value::object([(
        "data",
        Value::Array(vec![
            Value::object([
                ("id", Value::string("fixture/no-tools")),
                ("name", Value::string("No tools")),
                ("supported_parameters", Value::Array(vec![])),
            ]),
            Value::object([
                ("id", Value::string("fixture/model")),
                ("name", Value::string("Fixture Model")),
                (
                    "supported_parameters",
                    Value::Array(vec![Value::string("tools")]),
                ),
                (
                    "pricing",
                    Value::object([
                        ("prompt", Value::string("0.000001")),
                        ("completion", Value::string("0.000002")),
                    ]),
                ),
            ]),
        ]),
    )])
}

#[test]
fn guided_setup_validates_searches_saves_and_reopens_without_environment() {
    let directory = Directory::new();
    let store = Store::new(directory.path().join(".jecode"));
    let fixture = HttpFixture::new(vec![(200, key_response()), (200, catalog())]);
    let mut ui = Prompts::new(&["fixture & tools", "9", "1"], &["isolated-fixture-key"]);
    let saved = configure_with(&store, None, &mut ui, |key| {
        Api::new(key.into())?;
        Ok(Api::fixture(&fixture.endpoint))
    })
    .unwrap()
    .unwrap();
    assert_eq!(saved.model, "fixture/model");
    assert_eq!(store.load().unwrap().unwrap().model, "fixture/model");
    assert!(!ui.messages.contains("isolated-fixture-key"));
    assert!(ui.messages.contains("$1.00 input / $2.00 output"));
    assert!(!ui.messages.contains("No tools"));
    assert!(ui.messages.contains("Saved."));
    let requests = fixture.finish();
    assert!(requests[0].headers[0].starts_with("GET /key "));
    assert!(
        requests[1].headers[0]
            .contains("supported_parameters=tools&q=fixture%20%26%20tools&limit=10")
    );
    assert!(requests.iter().all(|request| request.body == Value::Null));
    assert!(requests.iter().all(|request| {
        request
            .headers
            .iter()
            .any(|header| header == "Authorization: Bearer isolated-fixture-key")
    }));
}

#[test]
fn cancelling_or_rejecting_a_key_preserves_existing_settings() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/original".into()).unwrap();
    store.save(&settings).unwrap();
    let original = fs::read(store.path()).unwrap();
    let fixture = HttpFixture::new(vec![(
        401,
        Value::object([(
            "error",
            Value::object([("message", Value::string("Rejected isolated-fixture-key"))]),
        )]),
    )]);
    let mut ui = Prompts::new(
        &[],
        &["invalid whitespace key", "isolated-fixture-key", "/cancel"],
    );
    assert!(
        configure_with(&store, Some(&settings), &mut ui, |key| {
            Api::new(key.into())?;
            Ok(Api::fixture(&fixture.endpoint))
        })
        .unwrap()
        .is_none()
    );
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(ui.messages.contains("HTTP 401"));
    assert!(ui.messages.contains("[redacted]"));
    assert!(!ui.messages.contains("isolated-fixture-key"));
    assert!(!ui.messages.contains("invalid whitespace key"));
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn keeping_settings_and_manual_selection_do_not_require_a_catalog() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/original".into()).unwrap();
    store.save(&settings).unwrap();
    let fixture = HttpFixture::new(vec![(200, key_response()), (200, key_response())]);
    let mut keep = Prompts::new(&[""], &[""]);
    let saved = configure_with(&store, Some(&settings), &mut keep, |_| {
        Ok(Api::fixture(&fixture.endpoint))
    })
    .unwrap()
    .unwrap();
    assert_eq!(saved.model, "fixture/original");
    let mut change = Prompts::new(&["=fixture/new"], &[""]);
    let saved = configure_with(&store, Some(&settings), &mut change, |_| {
        Ok(Api::fixture(&fixture.endpoint))
    })
    .unwrap()
    .unwrap();
    assert_eq!(saved.model, "fixture/new");
    assert_eq!(store.load().unwrap().unwrap().model, "fixture/new");
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn catalog_failure_can_be_retried_and_cancelled_without_saving() {
    let directory = Directory::new();
    let store = Store::new(directory.path().to_path_buf());
    let fixture = HttpFixture::new(vec![
        (200, key_response()),
        (503, Value::object([])),
        (200, catalog()),
    ]);
    let mut ui = Prompts::new(
        &["fixture", "fixture", "/cancel"],
        &["isolated-fixture-key"],
    );
    assert!(
        configure_with(&store, None, &mut ui, |_| Ok(Api::fixture(
            &fixture.endpoint
        )))
        .unwrap()
        .is_none()
    );
    assert!(!store.path().exists());
    assert!(ui.messages.contains("HTTP 503"));
    assert!(ui.messages.contains("Setup cancelled"));
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn eof_at_first_prompt_leaves_no_directory_or_config() {
    let directory = Directory::new();
    let store = Store::new(directory.path().join("absent"));
    let mut ui = Prompts::new(&[], &[]);
    assert!(
        configure_with(&store, None, &mut ui, |_| panic!(
            "no API request after cancellation"
        ))
        .unwrap()
        .is_none()
    );
    assert!(!store.path().parent().unwrap().exists());
}
