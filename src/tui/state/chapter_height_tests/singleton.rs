use super::*;

#[test]
fn failed_command_singleton_keeps_height_with_a_bounded_failure_receipt() {
    for columns in [80, 40] {
        let mut state = State {
            width: columns,
            ..State::default()
        };
        state.begin_tool_turn();
        start(
            &mut state,
            "failed",
            "bash",
            Value::object([
                (
                    "command",
                    Value::string("printf very_long_argument_".repeat(12)),
                ),
                ("description", Value::string("Run parser checks")),
                ("work", Value::string("Inspect")),
            ]),
        );
        finish(
            &mut state,
            "failed",
            "bash",
            Value::object([
                ("exit_code", Value::number(101)),
                (
                    "stderr",
                    Value::string("raw-error-one\nraw-error-two\nraw-error-three\n"),
                ),
            ]),
        );
        let singleton = height(&state, false);
        let singleton_with_keycap = height(&state, true);
        assert!(
            singleton >= 6,
            "{columns} columns: singleton {singleton}\n{}",
            shown(&state)
        );
        start(
            &mut state,
            "second",
            "bash",
            Value::object([
                ("command", Value::string("printf second")),
                ("work", Value::string("Inspect")),
            ]),
        );
        let chapter = shown(&state);
        assert!(
            height(&state, false) >= singleton,
            "{columns} columns: {} < {singleton}\n{chapter}",
            height(&state, false)
        );
        assert!(
            height(&state, true) >= singleton_with_keycap,
            "{columns} columns: {} < {singleton_with_keycap} with keycap\n{chapter}",
            height(&state, true)
        );
        assert_eq!(chapter.matches("raw-error-one").count(), 1);
        assert!(!chapter.contains("raw-error-two"));
        assert!(!chapter.contains("raw-error-three"));
    }
}

#[test]
fn pending_browser_singleton_keeps_height_when_next_browser_starts() {
    for columns in [80, 40] {
        let mut state = State {
            width: columns,
            ..State::default()
        };
        state.begin_tool_turn();
        start(
            &mut state,
            "first",
            "browser",
            Value::object([
                ("action", Value::string("evaluate")),
                (
                    "expression",
                    Value::string("const value = browser.evaluate(); ".repeat(12)),
                ),
                ("work", Value::string("Inspect")),
            ]),
        );
        let singleton = height(&state, false);
        assert!(singleton >= 4, "{columns} columns: singleton {singleton}");
        start(
            &mut state,
            "second",
            "browser",
            Value::object([
                ("action", Value::string("snapshot")),
                ("work", Value::string("Inspect")),
            ]),
        );
        assert!(
            height(&state, false) >= singleton,
            "{columns} columns: {} < {singleton}\n{}",
            height(&state, false),
            shown(&state)
        );
        assert!(shown(&state).contains("Browser  snapshot"));
    }
}
