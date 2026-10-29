use super::*;

fn record(id: &str, number: usize, closed: bool) -> Value {
    Value::object([
        ("id", Value::string(id)),
        ("number", Value::number(number)),
        ("state", Value::string("returned")),
        ("workspace_closed", Value::Bool(closed)),
    ])
}

#[test]
fn a_closed_collaborators_number_is_shown_again_and_open_ones_keep_theirs() {
    let mut state = State::default();
    assert_eq!(state.free_number(), 1);
    for (id, number) in [("a1", 1), ("a2", 2)] {
        let record = Record::from_value(&record(id, number, false)).unwrap();
        state.records.push(Tracked::new(record));
    }
    assert_eq!(state.free_number(), 3, "both are open");
    state.record("a1").unwrap().workspace_closed = true;
    assert_eq!(state.free_number(), 1, "AGENT 1 closed");
    state.record("a2").unwrap().workspace_closed = true;
    assert_eq!(state.free_number(), 1);
}

#[test]
fn resumed_ids_continue_after_the_highest_sequence_not_the_shown_number() {
    assert_eq!(sequence(&record("a7", 1, true)), 7);
    assert_eq!(sequence(&record("a2", 2, false)), 2);
    assert_eq!(sequence(&Value::object([("number", Value::number(3))])), 3);
}
