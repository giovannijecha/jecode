use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
};

#[test]
fn resume_returns_open_reports_to_main_once_and_closed_history_stays_closed() {
    for reviewed in [false, true] {
        let project = Directory::new();
        let home = Directory::new();
        let review = Value::object([
            ("action", Value::string("review")),
            ("id", Value::string("a1")),
            ("report_version", Value::number(1)),
            ("note", Value::string("Checked the reported result")),
        ]);
        let close = Value::object([
            ("action", Value::string("close")),
            ("id", Value::string("a1")),
        ]);
        let mut replies = vec![(200, completion("Original child report", vec![]))];
        if !reviewed {
            replies.push((
                200,
                completion(
                    "Checked the result",
                    vec![tool_call("review", "agent", review.clone())],
                ),
            ));
        }
        replies.push((
            200,
            completion("", vec![tool_call("close", "agent", close)]),
        ));
        replies.push((200, completion("Assignment closed", vec![])));
        let expected_requests = replies.len();
        let fixture = HttpFixture::new(replies);
        let make_agent = || {
            Agent::new(
                OpenRouter::fixture(fixture.endpoint.clone()),
                Tools::new(project.path()).unwrap(),
            )
        };
        let mut original = make_agent();
        original.enable_sessions(home.path()).unwrap();
        original.agent_tool(&super::tests::create()).unwrap();
        original
            .collaboration()
            .wait(
                Some("a1"),
                Duration::from_secs(5),
                &crate::cancel::Cancellation::default(),
            )
            .unwrap();
        if reviewed {
            original.agent_tool(&review).unwrap();
        }
        let id = original.sessions().unwrap().id();
        original.save_session().unwrap();
        drop(original);

        let mut resumed = make_agent();
        resumed.enable_sessions(home.path()).unwrap();
        resumed.resume(&id).unwrap();
        assert!(resumed.has_open_collaboration_report());
        assert_eq!(
            resumed.collaboration().snapshot()[0].get("reviewed"),
            Some(&Value::Bool(reviewed)),
            "resume must not fabricate a review",
        );
        assert!(resumed.continue_collaboration(&mut |_| Ok(())).unwrap());
        assert!(!resumed.continue_collaboration(&mut |_| Ok(())).unwrap());
        assert!(resumed.collaboration().snapshot().is_empty());
        assert!(!resumed.has_open_collaboration_report());
        assert!(
            resumed
                .archive()
                .messages
                .lock()
                .unwrap()
                .iter()
                .all(|message| { message.get("role").and_then(Value::as_str) != Some("user") }),
            "continuation must not invent user input"
        );
        drop(resumed);

        let mut reopened = make_agent();
        reopened.enable_sessions(home.path()).unwrap();
        reopened.resume(&id).unwrap();
        assert!(!reopened.has_open_collaboration_report());
        assert!(!reopened.continue_collaboration(&mut |_| Ok(())).unwrap());
        assert!(reopened.collaboration().snapshot().is_empty());
        let requests = fixture.finish();
        assert_eq!(requests.len(), expected_requests);
        assert!(requests[1].body.encode().contains("Original child report"));
    }
}
