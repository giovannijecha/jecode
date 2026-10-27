use super::*;

#[test]
fn discovery_descriptions_and_summaries_show_targets_and_continuation() {
    let listing = Value::object([
        ("entries_returned", Value::number(3)),
        ("next_offset", Value::number(4)),
        ("complete", Value::Bool(false)),
        ("errors", Value::Array(vec![Value::object([])])),
    ]);
    assert_eq!(
        tool_summary("list", &listing),
        "3 entries; 1 error; more from 4"
    );
    let search = Value::object([
        ("matches_returned", Value::number(0)),
        ("files_searched", Value::number(7)),
        ("complete", Value::Bool(true)),
    ]);
    assert_eq!(tool_summary("search", &search), "0 matches in 7 files");
    let args = Value::object([
        ("query", Value::string("needle")),
        ("description", Value::string(" Find callers ")),
    ]);
    assert_eq!(description("search", &args), "Find callers");
    assert_eq!(target("search", &args), "needle in .");
    assert_eq!(target("list", &Value::object([])), ".");
}
