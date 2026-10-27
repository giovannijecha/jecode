use super::*;
use crate::tui::theme::{BADGE_BAD, BADGE_IDLE, BADGE_READ, BADGE_TEXT, S_BASE, Style, Surface};

/// The title badge: its ink and surface.
fn badge(rows: &[Line]) -> (Style, Option<Surface>) {
    (rows[0].spans[0].style, rows[0].spans[0].background)
}

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
    assert!(plain(&running).starts_with(" SKILL  review"));
    assert_eq!(running[0].spans[0].background, Some(BADGE_IDLE));
    let ink = &running[0].spans[1];
    assert_eq!((ink.text.as_str(), ink.style), ("SKILL", S_BASE.bold()));

    let result = Value::object([
        ("content", Value::string("1: # Review\n2: Check changes.\n")),
        ("offset", Value::number(1)),
        ("lines_returned", Value::number(2)),
    ]);
    let done = draw("skill:review", Some(&result), false);
    assert_eq!(badge(&done), (BADGE_TEXT.bold(), Some(BADGE_READ)));
    // The fact joins the one-row title.
    assert_eq!(plain(&done), " SKILL  review  Loaded skill · 2 lines");
    assert_eq!(done.len(), 1);
    let expanded = draw("skill:review", Some(&result), true);
    assert!(plain(&expanded).contains("2   Check changes."));

    let failed_result = Value::object([("error", Value::string("Skill unavailable"))]);
    let failed = draw("skill:review", Some(&failed_result), false);
    assert_eq!(badge(&failed), (BADGE_TEXT.bold(), Some(BADGE_BAD)));
    assert!(plain(&failed).contains("review  Skill unavailable"));
    assert!(!plain(&failed).contains("Loaded skill"));

    let resource = draw("skill:review/references/CHECKLIST.md", Some(&result), false);
    assert!(plain(&resource).starts_with(" READ  skill:review/references/CHECKLIST.md"));
    assert!(!plain(&resource).contains("Loaded skill"));

    let explicit_root = draw("skill:review/SKILL.md", Some(&result), false);
    assert!(plain(&explicit_root).starts_with(" SKILL  review"));
    assert!(plain(&explicit_root).contains("Loaded skill"));
}
