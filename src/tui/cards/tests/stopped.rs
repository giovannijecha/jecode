use super::*;
use crate::tui::theme::{BADGE_WARN, Style, WARNING};

const REASON: &str = "The user denied editing session.rs; nothing ran. Continue without it.";
const TOLD: &str = "Denied · nothing ran; the agent was told";
const REFUSED: &str = "The permission level changed to Read while this call waited; nothing ran";

fn denied() -> Value {
    Value::object([
        ("outcome", Value::string("denied")),
        ("reason", Value::string(REASON)),
    ])
}

fn refused() -> Value {
    Value::object([
        ("outcome", Value::string("unavailable")),
        ("reason", Value::string(REFUSED)),
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
                (
                    "edits",
                    Value::Array(vec![Value::object([
                        ("old_text", Value::string("before")),
                        ("new_text", Value::string("after")),
                    ])]),
                ),
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

fn ink(rows: &[crate::tui::line::Line], words: &str) -> Option<Style> {
    rows.iter()
        .flat_map(|row| &row.spans)
        .find(|span| span.text.contains(words))
        .map(|span| span.style)
}

#[test]
fn a_call_that_never_ran_is_stopped_says_why_and_shows_no_work() {
    for (result, says) in [(denied(), TOLD), (refused(), REFUSED)] {
        for (name, arguments) in calls() {
            for expanded in [false, true] {
                let rows = draw(
                    name,
                    &arguments,
                    Some(&result),
                    None,
                    100,
                    &Presentation {
                        expanded,
                        ..Presentation::default()
                    },
                );
                let text = plain(&rows);
                assert_eq!(rows[0].spans[0].background, Some(BADGE_WARN), "{text}");
                // The first segment is the marker; its detail remains quiet.
                let marker = if says.starts_with("Denied") {
                    "Denied"
                } else {
                    "Not run"
                };
                assert!(
                    rows.iter()
                        .flat_map(|row| &row.spans)
                        .any(|span| span.style == WARNING && span.text.contains(marker)),
                    "{name}: {text}"
                );
                if says.contains(" · ") {
                    assert_eq!(ink(&rows, "nothing ran; the agent was told"), Some(DIM));
                }
                for invented in [
                    "+1",
                    "−1",
                    "Wrote",
                    "0 lines",
                    "Done",
                    "after",
                    "\"outcome\"",
                ] {
                    assert!(
                        !text.contains(invented),
                        "{name} invented {invented}: {text}"
                    );
                }
                // What the model is told to do next is not for the person.
                assert!(!text.contains("Continue without it"), "{name}: {text}");
            }
        }
    }
}
