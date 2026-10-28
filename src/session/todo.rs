use crate::{
    agent::Agent,
    json::Value,
    todo::{self, Phase, Plan},
};
use std::io::Write;

/// The `/todo` menu as text: the plan in order, each task with its owner and
/// state, without a model request.
pub(super) fn print(agent: &Agent, output: &mut impl Write) -> Result<(), String> {
    let text = match agent.todo_plan() {
        None => "No plan yet. The model writes one for multi-step work.".into(),
        Some(plan) => render(&plan, &agent.collaboration().snapshot()),
    };
    writeln!(output, "{}", agent.redact(&text)).map_err(|error| error.to_string())
}

fn render(plan: &Plan, records: &[Value]) -> String {
    let states: Vec<_> = plan
        .tasks
        .iter()
        .map(|task| {
            let owner = todo::owner(task, records);
            (task, Phase::of(task, owner), todo::owner_label(task, owner))
        })
        .collect();
    let counted = states
        .iter()
        .filter(|(_, phase, _)| *phase != Phase::Cancelled)
        .count();
    let done = states
        .iter()
        .filter(|(_, phase, _)| *phase == Phase::Done)
        .count();
    let mut text = format!("Todo · {}", plan.title);
    if counted > 0 {
        text.push_str(&format!(" · {done} of {counted} done"));
    }
    if plan.tasks.is_empty() {
        text.push_str("\nThis plan has no tasks.");
    }
    let mut number = 0;
    for (task, phase, owner) in states {
        let column = if phase == Phase::Cancelled {
            "  ".to_owned()
        } else {
            number += 1;
            format!("{number:>2}")
        };
        let word = match phase {
            Phase::NotStarted => "not started",
            phase => phase.word(false),
        };
        let parts = [task.title.as_str(), owner.as_deref().unwrap_or(""), word];
        let parts = parts.iter().filter(|part| !part.is_empty());
        text.push_str(&format!(
            "\n{column}  {}",
            parts.copied().collect::<Vec<_>>().join("  ")
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todo::Task;

    fn task(id: &str, status: &str, agent: Option<&str>) -> Task {
        Task {
            id: id.into(),
            title: format!("Task {id}"),
            status: status.into(),
            note: "notes stay for the model".into(),
            agent: agent.map(Into::into),
        }
    }

    #[test]
    fn plain_menu_lists_states_owners_and_unnumbered_cancelled_tasks() {
        let plan = Plan {
            title: "CSV export".into(),
            revision: 3,
            tasks: vec![
                task("t1", "done", None),
                task("t2", "in_progress", Some("a1")),
                task("t3", "pending", Some("a2")),
                task("t4", "cancelled", None),
                task("t5", "pending", None),
            ],
        };
        let records = [
            Value::object([
                ("id", Value::string("a1")),
                ("number", Value::number(1)),
                ("state", Value::string("returned")),
            ]),
            Value::object([
                ("id", Value::string("a2")),
                ("number", Value::number(2)),
                ("state", Value::string("failed")),
            ]),
        ];
        assert_eq!(
            render(&plan, &records),
            "Todo · CSV export · 1 of 4 done\n \
             1  Task t1  done\n \
             2  Task t2  AGENT 1  awaiting review\n \
             3  Task t3  AGENT 2  failed\n    \
             Task t4  cancelled\n \
             4  Task t5  not started"
        );
        assert!(!render(&plan, &records).contains("notes stay"));
    }
}
