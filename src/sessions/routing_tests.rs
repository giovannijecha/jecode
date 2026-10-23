use super::*;
use crate::{
    backend::openrouter::Routing,
    effort::Effort,
    json::{self, Value},
    test_support::Directory,
};

#[test]
fn routing_round_trips_and_older_sessions_keep_sending_no_routing_fields() {
    let directory = Directory::new();
    let mut saved = Document::new(
        directory.path().to_path_buf(),
        "fixture/model".into(),
        Effort::High,
    )
    .unwrap();
    saved.messages = ["system", "user", "assistant"]
        .map(|role| {
            Value::object([
                ("role", Value::string(role)),
                ("content", Value::string(role)),
            ])
        })
        .into();
    assert_eq!(saved.routing, Routing::Auto);
    saved.routing = Routing::Endpoint("deepinfra/fp8".into());
    let mut value = saved.value();
    assert_eq!(Document::parse(&value).unwrap().routing, saved.routing);
    let Value::Object(fields) = &mut value else {
        unreachable!()
    };
    // A mode from a newer version is not guessed at.
    fields.insert(
        "routing".into(),
        json::parse(r#"{"mode":"cheapest"}"#).unwrap(),
    );
    assert_eq!(Document::parse(&value).unwrap().routing, Routing::Default);
    let Value::Object(fields) = &mut value else {
        unreachable!()
    };
    fields.remove("routing");
    let legacy = Document::parse(&value).unwrap();
    assert_eq!(legacy.routing, Routing::Default);
    assert_eq!(
        legacy.state_value().get("routing").unwrap().encode(),
        r#"{"mode":"default"}"#
    );
}
