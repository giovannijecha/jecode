use crate::session::{
    TranscriptItem,
    history::{History, Turn},
};
impl History {
    pub(crate) fn transcript(&self) -> Vec<TranscriptItem> {
        let mut items = Vec::new();
        if self.base_turn != 0 || self.base_step != 0 {
            let summary = self
                .projection
                .summary
                .chars()
                .take(4096)
                .collect::<String>();
            items.push(TranscriptItem {
                role: "Status",
                text: format!(
                    "Earlier canonical history is retained on disk ({} earlier turns, {} earlier steps in this turn). Context summary: {}",
                    self.base_turn,
                    self.base_step,
                    summary,
                ),
            });
        }
        items.extend(turn_items(&self.turns));
        items
    }
}
pub(super) fn turn_items(turns: &[Turn]) -> Vec<TranscriptItem> {
    let mut items = Vec::new();
    for turn in turns {
        items.push(TranscriptItem {
            role: "You",
            text: turn.prompt.clone(),
        });
        for index in 0..=turn.steps.len() {
            items.extend(
                turn.guidance
                    .iter()
                    .filter(|g| g.after_step == index)
                    .map(|g| TranscriptItem {
                        role: "You",
                        text: g.text.clone(),
                    }),
            );
            let Some(step) = turn.steps.get(index) else {
                continue;
            };
            if !step.text.is_empty() {
                items.push(TranscriptItem {
                    role: "Assistant",
                    text: step.text.clone(),
                });
            }
            for result in &step.results {
                items.push(TranscriptItem {
                    role: "Tool",
                    text: result.summary.clone(),
                });
            }
        }
        items.push(TranscriptItem {
                role: "Status",
                text: if turn.end.is_none() {
                    "Interrupted session / no operation was replayed. Check any unknown tool outcome before continuing.".into()
                } else { turn.outcome.clone() },
            });
    }
    items
}
