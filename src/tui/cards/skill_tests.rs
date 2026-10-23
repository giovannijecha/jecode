use super::*;

fn draw(path: &str, result: Option<&Value>, expanded: bool) -> Vec<Line> {
    render(
        Tool {
            name: "read",
            arguments: &Value::object([("path", Value::string(path))]),
            summary: "complete",
            result,
            change: None,
            presentation: &Presentation {
                expanded,
                ..Presentation::default()
            },
        },
        80,
    )
}

fn plain(rows: &[Line]) -> String {
    rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n")
}

#[test]
fn skill_root_reads_use_tool_card_states_and_resource_reads_keep_their_path() {
    let running = draw("skill:review", None, false);
    assert!(plain(&running).contains("Skill(review)"));
    assert_eq!(running[0].spans[0].style, TOOL_ACCENT);

    let result = Value::object([
        ("content", Value::string("1: # Review\n2: Check changes.\n")),
        ("offset", Value::number(1)),
        ("lines_returned", Value::number(2)),
    ]);
    let done = draw("skill:review", Some(&result), false);
    assert_eq!(done[0].spans[0].style, GOOD);
    assert!(plain(&done).contains("Loaded skill · Read 2 lines"));
    assert_eq!(done.len(), 2);
    let expanded = draw("skill:review", Some(&result), true);
    assert!(plain(&expanded).contains("2   Check changes."));

    let failed_result = Value::object([("error", Value::string("Skill unavailable"))]);
    let failed = draw("skill:review", Some(&failed_result), false);
    assert_eq!(failed[0].spans[0].style, BAD);
    assert!(plain(&failed).contains("Failed · Skill unavailable"));
    assert!(!plain(&failed).contains("Loaded skill"));

    let resource = draw("skill:review/references/CHECKLIST.md", Some(&result), false);
    assert!(plain(&resource).contains("Read(skill:review/references/CHECKLIST.md)"));
    assert!(!plain(&resource).contains("Loaded skill"));

    let explicit_root = draw("skill:review/SKILL.md", Some(&result), false);
    assert!(plain(&explicit_root).contains("Skill(review)"));
    assert!(plain(&explicit_root).contains("Loaded skill"));
}
