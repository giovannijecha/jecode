use crate::agent::Agent;
use std::io::Write;

pub(super) fn print(agent: &Agent, output: &mut impl Write) -> Result<(), String> {
    let text = match agent.todo_plan() {
        None => {
            "No plan in this conversation. The model maintains the plan through todo_list.".into()
        }
        Some(plan) => {
            let mut text = format!(
                "TODO · {} · revision {} · read only\n",
                plan.title, plan.revision
            );
            if plan.tasks.is_empty() {
                text.push_str("No tasks.\n");
            }
            for task in plan.tasks {
                text.push_str(&format!("{} · {} · {}\n", task.id, task.status, task.title));
                if !task.note.is_empty() {
                    text.push_str(&format!("  {}\n", task.note));
                }
                if let Some(agent) = task.agent {
                    text.push_str(&format!("  Agent: {agent}\n"));
                }
            }
            text
        }
    };
    writeln!(output, "{}", agent.redact(&text)).map_err(|error| error.to_string())
}
