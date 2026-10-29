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

#[test]
fn map_targets_and_summaries_cover_overview_and_note_actions() {
    let args = Value::object([
        ("query", Value::string("auth")),
        ("path", Value::string("src")),
        ("description", Value::string(" Trace auth ")),
    ]);
    assert_eq!(description("map", &args), "Trace auth");
    assert_eq!(target("map", &args), "auth in src");
    assert_eq!(target("map", &Value::object([])), ".");
    let overview = Value::object([
        ("entries_returned", Value::number(2)),
        ("files", Value::number(8)),
        ("notes_count", Value::number(1)),
        ("errors_count", Value::number(1)),
        ("next_offset", Value::number(3)),
    ]);
    assert_eq!(
        tool_summary("map", &overview),
        "2 entries in 8 files; 1 note; 1 error; more from 3"
    );
    let truncated = Value::object([
        ("entries_returned", Value::number(1)),
        ("inventory_truncated", Value::Bool(true)),
        ("complete", Value::Bool(true)),
    ]);
    assert_eq!(tool_summary("map", &truncated), "1 entry; partial");
    let remember = Value::object([("action", Value::string("remember"))]);
    let forget = Value::object([
        ("action", Value::string("forget")),
        ("removed", Value::Bool(false)),
    ]);
    assert_eq!(tool_summary("map", &remember), "Note saved");
    assert_eq!(tool_summary("map", &forget), "Note not found");
}
