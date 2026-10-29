use super::*;
use crate::{
    backend::{Model, Routing, openrouter::OpenRouter},
    effort::Effort,
    json::Value,
    test_support::{Directory, HttpFixture},
    tools::Tools,
};
use std::collections::BTreeMap;

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        super::tests::config(directory),
        None,
    )
}

#[test]
fn held_model_changes_preview_the_routing_that_replay_will_apply() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let pin_a = Routing::Endpoint("provider/a".into());
    let pin_b = Routing::Endpoint("provider/b".into());
    app.agent
        .as_mut()
        .unwrap()
        .set_routing_preferences(BTreeMap::from([
            ("fixture/model".into(), pin_a.clone()),
            ("fixture/other".into(), pin_b.clone()),
        ]));
    app.agent.as_mut().unwrap().set_routing(pin_a.clone());
    let agent = app.hold();

    app.defer(held::Deferred::Model {
        model: "fixture/other".into(),
        effort: Effort::High,
        services: None,
    });
    assert_eq!(app.facts().routing, pin_b);
    app.defer(held::Deferred::Model {
        model: "fixture/other".into(),
        effort: Effort::Low,
        services: None,
    });
    assert_eq!(app.facts().routing, pin_b, "effort alone keeps the pin");
    app.defer(held::Deferred::Model {
        model: "fixture/unpinned".into(),
        effort: Effort::Default,
        services: None,
    });
    assert_eq!(app.facts().routing, Routing::Auto);
    app.defer(held::Deferred::Model {
        model: "fixture/model".into(),
        effort: Effort::Default,
        services: None,
    });
    assert_eq!(
        app.facts().routing,
        pin_a,
        "later switches use their preference"
    );

    app.agent = Some(agent);
    app.apply_deferred();
    assert_eq!(app.agent.as_ref().unwrap().routing(), app.facts().routing);
    assert_eq!(app.agent.as_ref().unwrap().routing(), pin_a);
    assert!(fixture.finish().is_empty());
}

#[test]
fn saved_default_routing_precedes_a_later_held_model_switch() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.agent
        .as_mut()
        .unwrap()
        .set_routing_preferences(BTreeMap::new());
    let agent = app.hold();
    let pin = Routing::Endpoint("provider/other".into());
    app.defer(held::Deferred::DefaultRouting {
        backend: "openrouter".into(),
        preferences: BTreeMap::from([("fixture/other".into(), pin.clone())]),
    });
    app.defer(held::Deferred::Model {
        model: "fixture/other".into(),
        effort: Effort::Default,
        services: None,
    });
    assert_eq!(app.facts().routing, pin);
    app.agent = Some(agent);
    app.apply_deferred();
    assert_eq!(app.agent.as_ref().unwrap().routing(), pin);
    assert!(fixture.finish().is_empty());
}

#[test]
fn validated_key_is_used_by_later_menus_while_the_agent_is_held() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, Value::object([("data", Value::Array(vec![]))]))]);
    let mut app = app(&directory, &fixture);
    let _agent = app.hold();
    let services = app
        .facts()
        .services
        .with_credential("replacement-fixture-key".into())
        .unwrap();
    app.defer(held::Deferred::Key(services));
    app.open_model_providers(false);
    super::tests::wait_until(&mut app, |app| app.job.is_none());
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|header| header == "Authorization: Bearer replacement-fixture-key")
    );
}

#[test]
fn model_menu_marks_the_held_choice_as_current() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let _agent = app.hold();
    app.defer(held::Deferred::Model {
        model: "fixture/other".into(),
        effort: Effort::Default,
        services: None,
    });
    app.catalog = ["fixture/model", "fixture/other"]
        .into_iter()
        .map(|id| Model {
            id: id.into(),
            name: id.into(),
            prompt_price: None,
            completion_price: None,
            efforts: vec![Effort::Default],
        })
        .collect();
    let menu = app.models_menu(false);
    assert!(matches!(
        menu.options[1].badge,
        Some(selector::Badge::Current)
    ));
    assert!(menu.options[0].badge.is_none());
    assert!(fixture.finish().is_empty());
}
