use super::*;
use crate::tui::theme::BADGE_WARN;

const REASON: &str = "The user denied editing session.rs; nothing ran. Continue without it.";

fn denied() -> Value {
    Value::object([
        ("outcome", Value::string("denied")),
        ("reason", Value::string(REASON)),
    ])
}

/// Each single-file call a level or the user can stop, with arguments that
/// would otherwise draw a diff or a written file.
fn calls() -> Vec<(&'static str, Value)> {
    vec![
        (
            "edit",
            Value::object([
                ("path", Value::string("session.rs")),
                ("old_text", Value::string("before")),
                ("new_text", Value::string("after")),
            ]),
        ),
        (
            "write",
            Value::object([
                ("path", Value::string("session.rs")),
                ("content", Value::string("after")),
            ]),
        ),
        ("read", args("path", "session.rs")),
        ("bash", args("command", "cargo test")),
    ]
}

#[test]
fn a_call_that_never_ran_is_stopped_says_why_and_shows_no_work() {
    let result = denied();
    for (name, arguments) in calls() {
        let collapsed = draw(
            name,
            &arguments,
            Some(&result),
            None,
            100,
            &Presentation::default(),
        );
        assert_eq!(
            collapsed[0].spans[0].background,
            Some(BADGE_WARN),
            "{name}: {}",
            plain(&collapsed)
        );
        let text = plain(&collapsed);
        for invented in ["+1", "−1", "Wrote", "0 lines", "Done", "after"] {
            assert!(
                !text.contains(invented),
                "{name} invented {invented}: {text}"
            );
        }
        assert!(text.contains("The user denied"), "{name}: {text}");

        let opened = plain(&draw(
            name,
            &arguments,
            Some(&result),
            None,
            100,
            &Presentation {
                expanded: true,
                ..Presentation::default()
            },
        ));
        assert!(opened.contains(REASON), "{name}: {opened}");
        assert!(!opened.contains("\"outcome\""), "{name}: {opened}");
    }
}
