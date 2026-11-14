use super::*;
use crate::events::Event;
use crate::tui::{cards, line::Line};

fn start(state: &mut State, id: &str, name: &str, arguments: Value) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: name.into(),
        arguments,
    });
}

fn finish(state: &mut State, id: &str, name: &str, result: Value) {
    state.event(Event::ToolFinished {
        id: id.into(),
        name: name.into(),
        summary: crate::events::tool_summary(name, &result),
        result,
    });
}

fn rendered(state: &State, keycap: bool) -> Vec<Vec<Line>> {
    let mut renderer = rows::Rows::default();
    (0..state.items.len())
        .map(|index| renderer.render_at(&state.items, index, state.width, keycap && index == 0))
        .collect()
}

fn work(name: &str, target: &str) -> Value {
    Value::object([
        ("work", Value::string("Inspect")),
        (name, Value::string(target)),
    ])
}

#[test]
fn compact_work_stays_within_eight_rows_even_after_a_tall_singleton() {
    for columns in [40, 80] {
        let mut state = State {
            width: columns,
            ..State::default()
        };
        state.begin_tool_turn();
        start(
            &mut state,
            "first",
            "bash",
            work("command", &"long command ".repeat(60)),
        );
        finish(
            &mut state,
            "first",
            "bash",
            Value::object([
                ("exit_code", Value::number(101)),
                ("stderr", Value::string("diagnostic\n".repeat(30))),
            ]),
        );
        start(&mut state, "second", "read", work("path", "src/second.rs"));
        finish(&mut state, "second", "read", Value::object([]));
        for index in 2..33 {
            let id = format!("call-{index}");
            start(
                &mut state,
                &id,
                "read",
                work("path", &format!("src/{index}.rs")),
            );
            finish(&mut state, &id, "read", Value::object([]));
        }
        let rows = rendered(&state, false);
        assert!(rows[0].len() <= 8, "{columns}: {:?}", rows[0]);
        assert!(rows[1..33].iter().all(Vec::is_empty));
        let plain = rows[0]
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!plain.contains("diagnostic"), "{plain}");
        assert_eq!(plain.matches("exit 101").count(), 1);
        assert!(plain.contains("33 calls"), "{plain}");
    }
}

#[test]
fn expanded_work_dissolves_into_identical_standalone_cards_with_blank_separators() {
    for columns in [40, 80] {
        let mut state = State {
            width: columns,
            ..State::default()
        };
        state.begin_tool_turn();
        start(&mut state, "first", "read", work("path", "src/first.rs"));
        finish(
            &mut state,
            "first",
            "read",
            Value::object([("lines_returned", Value::number(1))]),
        );
        start(
            &mut state,
            "second",
            "bash",
            work("command", "printf second"),
        );
        finish(
            &mut state,
            "second",
            "bash",
            Value::object([("exit_code", Value::number(0))]),
        );
        state.close_tools();
        state.finish_tool_turn();
        let compact = rendered(&state, true);
        assert!(compact[0].last().unwrap().plain().contains("Ctrl+O"));
        assert!(compact[1].is_empty());
        state.toggle_tools();
        let expanded = rendered(&state, true);
        assert!(expanded[0].iter().any(|row| row.plain().contains("READ")));
        assert!(
            expanded[1]
                .first()
                .is_some_and(|row| row.plain().is_empty())
        );
        let all = expanded
            .iter()
            .flatten()
            .map(Line::plain)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !all.contains("WORK") && !all.contains("calls folded") && !all.contains("files read"),
            "{all}"
        );
        assert!(all.contains("Ctrl+O collapse"), "{all}");
        let first = state.items[0].tool().unwrap();
        let second = state.items[1].tool().unwrap();
        let first_card = cards::card(first, columns, true);
        let second_card = cards::card(second, columns, false);
        assert_eq!(expanded[0], first_card.rows);
        assert_eq!(&expanded[1][1..], second_card.rows);
    }
}
