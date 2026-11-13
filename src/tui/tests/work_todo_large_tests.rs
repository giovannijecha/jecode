//! A plan past the former 40-task and text-size caps stays usable in /todo.
use super::*;
use crate::{
    todo::{Plan, Task},
    tui::{keys::Decoded, line::Line, terminal::Key, text},
};

fn plan(tasks: usize) -> Plan {
    Plan {
        title: format!("Migration {}", "long title ".repeat(40)),
        revision: 1,
        tasks: (0..tasks)
            .map(|index| Task {
                id: format!("t{index}"),
                title: format!("Task {index} {}", "detail ".repeat(60)),
                status: if index < 3 { "done" } else { "pending" }.into(),
                note: "note ".repeat(400),
                agent: None,
            })
            .collect(),
    }
}

#[test]
fn a_plan_of_many_long_tasks_keeps_numbers_aligned_and_scrolls_to_the_end() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.width = 80;
    app.state.height = 32;
    app.open_todo();
    app.state.todo_plan = Some(plan(120));

    let rows = view::todo::rows(app.state.todo_plan.as_ref().unwrap(), &[], 80);
    assert_eq!(rows.len(), 120);
    let row = |index: usize| rows[index].plain();
    assert!(row(0).starts_with("    1  ◉ Task 0"), "{}", row(0));
    assert!(row(119).starts_with("  120  ○ Task 119"), "{}", row(119));
    assert!(rows.iter().all(|row| text::cells(&row.plain()) <= 80));

    let frame = view::composer_frame(&app.state, "fixture", "folder");
    let rule = frame
        .live
        .iter()
        .map(Line::plain)
        .find(|row| row.contains("Todo"))
        .unwrap();
    assert!(rule.ends_with(" 3 of 120 done ─"), "{rule}");
    assert!(
        !rule.contains('▰'),
        "a strip this long leaves only the count"
    );

    app.input(Decoded::Key(Key {
        code: 35,
        modifiers: 0,
        character: 0,
    }))
    .unwrap();
    let frame = view::composer_frame(&app.state, "fixture", "folder");
    assert!(
        frame
            .live
            .iter()
            .any(|row| row.plain().contains("120  ○ Task 119")),
        "End shows the last task"
    );
    assert!(fixture.finish().is_empty());
}
