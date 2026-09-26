use crate::session::{
    TranscriptItem, TranscriptTool,
    history::{History, Turn},
};
impl History {
    pub(crate) fn transcript(&self) -> Vec<TranscriptItem> {
        let mut before = Vec::new();
        let mut warning = None;
        let mut parent = self.display_parent.clone();
        if let Some(record) = &self.record
            && parent.is_some()
        {
            let root = record.user_store();
            let mut seen = std::collections::BTreeSet::new();
            let directory = record.directory.as_deref();
            for _ in 0..16 {
                let Some(id) = parent.take() else { break };
                if !seen.insert(id.clone()) || id == record.id() {
                    warning = Some("Display ancestry contains a cycle".to_owned());
                    break;
                }
                let saved = root
                    .as_ref()
                    .ok()
                    .and_then(|store| super::load(store, &id, false).ok());
                let Some(saved) = saved.filter(|saved| {
                    saved.directory.as_deref() == directory.map(std::path::Path::new)
                }) else {
                    warning = Some(format!("Earlier session {id} is unavailable for display"));
                    break;
                };
                parent = saved.history.display_parent.clone();
                before.push((id, saved.history.transcript_local()));
            }
            if parent.is_some() {
                warning = Some("Display ancestry exceeds 16 session boundaries".into());
            }
        }
        let mut items = Vec::new();
        if let Some(warning) = warning {
            items.push(TranscriptItem {
                role: "Status",
                text: warning,
                tool: None,
            });
        }
        let mut previous = None;
        for (id, earlier) in before.into_iter().rev() {
            if let Some(parent) = previous.take() {
                items.push(TranscriptItem {
                    role: "ClearBoundary",
                    text: format!("{parent}\n{id}"),
                    tool: None,
                });
            }
            items.extend(earlier);
            previous = Some(id);
        }
        if let (Some(parent), Some(record)) = (previous, self.record.as_ref()) {
            items.push(TranscriptItem {
                role: "ClearBoundary",
                text: format!("{parent}\n{}", record.id()),
                tool: None,
            });
        }
        items.extend(self.transcript_local());
        items
    }

    fn transcript_local(&self) -> Vec<TranscriptItem> {
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
                tool: None,
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
            tool: None,
        });
        for index in 0..=turn.steps.len() {
            items.extend(
                turn.guidance
                    .iter()
                    .filter(|g| g.after_step == index)
                    .map(|g| TranscriptItem {
                        role: "You",
                        text: g.text.clone(),
                        tool: None,
                    }),
            );
            let Some(step) = turn.steps.get(index) else {
                continue;
            };
            if !step.text.is_empty() {
                items.push(TranscriptItem {
                    role: "Assistant",
                    text: step.text.clone(),
                    tool: None,
                });
            }
            for (index, result) in step.results.iter().enumerate() {
                let call = step
                    .response
                    .as_ref()
                    .and_then(|response| response.tool_calls.get(index));
                let prepared = call.and_then(|call| {
                    crate::tools::Prepared::parse(&call.name, &call.arguments).ok()
                });
                let name = prepared.as_ref().map_or_else(
                    || call.map_or_else(|| "tool".to_owned(), |call| call.name.clone()),
                    |prepared| prepared.name().to_owned(),
                );
                let subject = prepared.as_ref().map_or_else(
                    || result.summary.clone(),
                    |prepared| prepared.path().to_owned(),
                );
                let (failed, limited, outcome_unknown) = receipt_state(&result.output);
                items.push(TranscriptItem {
                    role: "Tool",
                    text: result.summary.clone(),
                    tool: Some(TranscriptTool {
                        name,
                        subject,
                        summary: result.summary.clone(),
                        output: result.output.clone(),
                        failed,
                        limited,
                        outcome_unknown,
                    }),
                });
            }
        }
        items.push(TranscriptItem {
                role: "Status",
                text: if turn.end.is_none() {
                    "Interrupted session / no operation was replayed. Check any unknown tool outcome before continuing.".into()
                } else { turn.outcome.clone() },
                tool: None,
            });
    }
    items
}

fn receipt_state(output: &str) -> (bool, bool, bool) {
    let Ok(value) = crate::json::parse(
        output,
        crate::json::Limits {
            bytes: output.len().max(1),
            nodes: 100_000,
            depth: 64,
        },
    ) else {
        return (false, false, true);
    };
    let outcome = value.get("ok");
    let failed = matches!(outcome, Some(crate::json::Value::Bool(false)));
    let outcome_unknown = !matches!(outcome, Some(crate::json::Value::Bool(_)));
    let limited = matches!(value.get("truncated"), Some(crate::json::Value::Bool(true)))
        || value
            .get("omitted")
            .and_then(crate::json::Value::unsigned)
            .is_some_and(|n| n > 0);
    (failed, limited, outcome_unknown)
}

#[cfg(test)]
mod tests {
    use super::receipt_state;

    #[test]
    fn historical_receipt_outcome_is_known_only_from_a_boolean_ok() {
        assert_eq!(receipt_state("{\"ok\":true}"), (false, false, false));
        assert_eq!(receipt_state("{\"ok\":false}"), (true, false, false));
        assert_eq!(receipt_state("{\"truncated\":true}"), (false, true, true));
        assert_eq!(receipt_state("older opaque format"), (false, false, true));
    }
}
