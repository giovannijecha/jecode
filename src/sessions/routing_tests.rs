use super::*;
use crate::{
    backend::openrouter::Routing,
    effort::Effort,
    json::{self, Value},
    test_support::Directory,
};

#[test]
fn routing_round_trips_and_invalid_preferences_are_rejected() {
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
    // Unknown routing must not silently change a resumed request.
    fields.insert(
        "routing".into(),
        json::parse(r#"{"mode":"cheapest"}"#).unwrap(),
    );
    assert!(Document::parse(&value).is_err());
    let Value::Object(fields) = &mut value else {
        unreachable!()
    };
    fields.remove("routing");
    assert!(Document::parse(&value).is_err());
}
