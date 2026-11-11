//! The Claude-plan route maps eligible state to mid-conversation system turns.
//! Continuations must keep valid roles and a stable prefix through resume.

use super::*;
use crate::auth::anthropic::Session;

const MODEL: &str = "claude-opus-5-5";

fn catalog() -> Response {
    let Response::Json(status, mut value) = super::catalog() else {
        unreachable!()
    };
    let Value::Object(fields) = &mut value else {
        unreachable!()
    };
    let Some(Value::Array(models)) = fields.get_mut("data") else {
        unreachable!()
    };
    let Value::Object(model) = &mut models[0] else {
        unreachable!()
    };
    model.insert("id".into(), Value::string(MODEL));
    model.insert("display_name".into(), Value::string("Claude Opus 5.5"));
    Response::Json(status, value)
}

fn agent(fixture: &HttpFixture, directory: &Directory, home: &Directory) -> Agent {
    let api = crate::backend::anthropic::Api::fixture(
        Session::fixture("fixture-anthropic-token"),
        fixture.endpoint.trim_end_matches("/chat/completions"),
    );
    assert_eq!(Service::id(&api), "anthropic");
    let mut agent = Agent::with_backend(
        api.create(MODEL.into()).unwrap(),
        Tools::new(directory.path()).unwrap(),
    );
    agent.set_effort(crate::effort::Effort::Medium);
    agent.enable_sessions(home.path()).unwrap();
    agent
}

fn assert_system_placement(body: &Value) {
    let messages = messages(body);
    let role = |message: &Value| {
        message
            .get("role")
            .and_then(Value::as_str)
            .unwrap()
            .to_owned()
    };
    for (index, message) in messages.iter().enumerate() {
        if role(message) == "system" {
            assert!(index > 0, "A system message cannot be first");
            assert!(matches!(
                role(&messages[index - 1]).as_str(),
                "user" | "system"
            ));
            if let Some(next) = messages.get(index + 1) {
                assert!(matches!(role(next).as_str(), "assistant" | "system"));
            }
        }
    }
}

#[test]
fn subscription_continuation_keeps_valid_roles_and_signed_prefix_through_resume() {
    let directory = Directory::new();
    let home = Directory::new();
    screenshots(&directory);
    let fixture = HttpFixture::streaming(vec![
        catalog(),
        reply(&[signed("", "sig-1"), text("Initial inspection complete.")]),
        reply(&[signed("", "sig-2"), read("toolu_a", r#"{"path":"a.png"}"#)]),
        reply(&[signed("", "sig-3"), text("Follow-up complete.")]),
    ]);
    let mut initial = agent(&fixture, &directory, &home);
    initial
        .run_turn("Inspect the page", &mut |_| Ok(()))
        .unwrap();
    initial.continue_turn(&mut |_| Ok(())).unwrap();
    let id = initial.sessions().unwrap().id();
    drop(initial);
    let requests = bodies(fixture);
    assert_eq!(requests.len(), 3);
    // Prove this fixture takes the subscription-only mapping branch.
    assert_eq!(
        messages(&requests[0])
            .last()
            .unwrap()
            .get("role")
            .and_then(Value::as_str),
        Some("system")
    );
    assert_eq!(
        messages(&requests[1])
            .last()
            .unwrap()
            .get("role")
            .and_then(Value::as_str),
        Some("user")
    );
    for pair in requests.windows(2) {
        assert_append_only(&pair[0], &pair[1], "subscription continuation");
    }
    for request in &requests {
        assert_system_placement(request);
    }

    let resumed_fixture = HttpFixture::streaming(vec![
        catalog(),
        reply(&[signed("", "sig-4"), read("toolu_b", r#"{"path":"b.png"}"#)]),
        reply(&[signed("", "sig-5"), text("Resumed follow-up complete.")]),
    ]);
    let mut resumed = agent(&resumed_fixture, &directory, &home);
    resumed.resume(&id).unwrap();
    resumed.continue_turn(&mut |_| Ok(())).unwrap();
    let after = bodies(resumed_fixture);
    assert_eq!(after.len(), 2);
    assert_append_only(requests.last().unwrap(), &after[0], "subscription resume");
    assert_append_only(&after[0], &after[1], "subscription resumed tool result");
    for request in &after {
        assert_system_placement(request);
    }
    assert_eq!(images(&after[1]).len(), 2);
    assert_eq!(reasoning(&after[1]), ["sig-1", "sig-2", "sig-3", "sig-4"]);
}
