//! Canonical attempts and tool receipts; only validated, paired items are projected.
use super::{End, Failure, Metrics, Model};
use crate::image::{Evidence, Images, data_url};
use crate::providers::openai_account::{Input, Request, Response, Status, client::Attempt};

pub(super) const MAX_TEXT: usize = 1024 * 1024;
pub(super) const MAX_CONTEXT: usize = 2 * 1024 * 1024;
pub(super) const MAX_REQUEST: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Step {
    pub text: String,
    pub reasoning: String,
    pub response: Option<Response>,
    pub results: Vec<Receipt>,
    pub accepted: bool,
    pub attempts: Vec<Attempt>,
}
impl Step {
    pub fn interrupted_reference(&self, outcome: &str) -> String {
        let excerpt = self.text.chars().take(16_384).collect::<String>();
        let last = self.attempts.last();
        format!(
            "Recorded unfinished generation (reference data, not a completed assistant response): outcome={outcome}; delivery={}; visible text={excerpt:?}{}; completed tool receipts remain in earlier validated steps. Continue from the recorded work without assuming an interrupted request failed remotely.",
            last.map_or("unknown", |attempt| attempt.delivery.name()),
            if excerpt.len() < self.text.len() {
                " [excerpt shortened; full text remains in canonical history]"
            } else {
                ""
            },
        )
    }
}
pub(super) struct Receipt {
    pub call_id: String,
    pub output: String,
    pub summary: String,
    pub image: Option<Evidence>,
}
pub(super) struct Turn {
    pub prompt: String,
    pub steps: Vec<Step>,
    pub end: Option<End>,
    pub outcome: String,
    pub metrics: Metrics,
    pub guidance: Vec<super::queue::Guidance>,
}
#[derive(Default)]
pub(super) struct History {
    pub turns: Vec<Turn>,
    /// Number of canonical turns kept only in the incremental log.
    pub base_turn: usize,
    /// Completed steps of the first resident turn retained only in the log.
    pub base_step: usize,
    pub base_guidance: usize,
    pub record: Option<super::persistence::Record>,
    #[cfg(test)]
    pub test_recovery: Option<crate::state::Store>,
    pub projection: super::context::Projection,
    /// Derived from the active workspace; not a second canonical permissions store.
    pub environment: String,
    pub shell: crate::command::Shell,
    /// Ephemeral, explicit account-catalog evidence for the selected model.
    pub image_capable: bool,
    #[cfg(test)]
    pub fail_next_checkpoint: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    pub checkpoint_calls: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub fail_checkpoint_from: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub effect_gate: Option<super::worker::EffectGate>,
    #[cfg(test)]
    pub test_outcome_checkpoint: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
impl History {
    pub fn images(&self) -> Result<Images, Failure> {
        if let Some(record) = &self.record {
            if record.legacy() {
                return Err(Failure::Storage);
            }
            return Images::in_store(
                &record.user_store().map_err(|_| Failure::Storage)?,
                record.id(),
            )
            .map_err(|_| Failure::Storage);
        }
        #[cfg(test)]
        if let Some(store) = &self.test_recovery {
            return Images::in_store(store, "s-00000000").map_err(|_| Failure::Storage);
        }
        Err(Failure::Storage)
    }
    pub fn can_view_images(&self) -> bool {
        self.image_capable && self.record.as_ref().is_none_or(|record| !record.legacy())
    }
    pub fn has_retained_images(&self) -> bool {
        self.turns.iter().any(|turn| {
            turn.steps
                .iter()
                .any(|step| step.results.iter().any(|receipt| receipt.image.is_some()))
        }) || self.projection.summary.contains("image_id")
    }
    pub fn pending_image(&self) -> bool {
        self.turns
            .last()
            .filter(|turn| turn.end.is_none())
            .and_then(|turn| turn.steps.last())
            .is_some_and(|step| {
                step.accepted && step.results.iter().any(|receipt| receipt.image.is_some())
            })
    }
    pub fn recovery_store(&self) -> std::io::Result<crate::workspace::RecoveryStore> {
        if let Some(record) = &self.record {
            return crate::workspace::RecoveryStore::in_store(&record.user_store()?);
        }
        #[cfg(test)]
        if let Some(store) = &self.test_recovery {
            return crate::workspace::RecoveryStore::in_store(store);
        }
        Err(std::io::ErrorKind::NotFound.into())
    }
    pub fn begin(&mut self, prompt: String) -> Result<(), Failure> {
        if prompt.len() > super::MAX_PROMPT_BYTES
            || self.record.as_ref().is_some_and(|record| record.legacy())
                && self.turn_count() >= 256
        {
            return Err(Failure::HistoryLimit);
        }
        self.turns.push(Turn {
            prompt,
            steps: Vec::new(),
            end: None,
            outcome: String::new(),
            metrics: Metrics::default(),
            guidance: Vec::new(),
        });
        Ok(())
    }
    pub fn turn_count(&self) -> usize {
        self.base_turn + self.turns.len()
    }
    /// The cursor that will exist after releasing the newly projected prefix.
    /// Guidance exactly at the step boundary remains resident.
    pub fn projected_cursor(&self) -> (usize, usize, usize) {
        let through = self.base_turn + self.projection.through;
        let step_base = if self.projection.through == 0 {
            self.base_step
        } else {
            0
        };
        let guidance_base = if self.projection.through == 0 {
            self.base_guidance
        } else {
            0
        };
        let covered_guidance = self.turns.get(self.projection.through).map_or(0, |turn| {
            turn.guidance
                .iter()
                .filter(|guidance| guidance.after_step < self.projection.step)
                .count()
        });
        (
            through,
            step_base + self.projection.step,
            guidance_base + covered_guidance,
        )
    }
    /// Release canonical turns already covered by a durable projection. Their
    /// bytes remain in the log and can be visited in bounded pages.
    pub fn release_projected(&mut self) {
        if self.record.as_ref().is_none_or(|record| record.legacy()) {
            return;
        }
        let (_, _, projected_guidance_base) = self.projected_cursor();
        let count = self.projection.through.min(self.turns.len());
        self.turns.drain(..count);
        self.base_turn += count;
        self.projection.through -= count;
        if count != 0 {
            self.base_step = 0;
            self.base_guidance = 0;
        }
        if let Some(turn) = self.turns.first_mut() {
            let steps = self.projection.step.min(turn.steps.len());
            turn.steps.drain(..steps);
            turn.guidance.retain(|g| g.after_step >= steps);
            self.base_guidance = projected_guidance_base;
            for guidance in &mut turn.guidance {
                guidance.after_step -= steps;
            }
            self.base_step += steps;
            self.projection.step -= steps;
        }
    }
    pub fn checkpoint(&self) -> Result<(), Failure> {
        #[cfg(test)]
        {
            let number = self
                .checkpoint_calls
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
                + 1;
            let from = self
                .fail_checkpoint_from
                .load(std::sync::atomic::Ordering::Acquire);
            if self
                .fail_next_checkpoint
                .swap(false, std::sync::atomic::Ordering::AcqRel)
                || from != 0 && number >= from
            {
                return Err(Failure::Storage);
            }
        }
        if let Some(record) = &self.record {
            record.save(self).map_err(|_| Failure::Storage)?;
        }
        Ok(())
    }
    pub fn request(&self, model: Model, workspace: bool) -> Result<Request, Failure> {
        let request = self.projected_request(model, workspace)?;
        request
            .encode(MAX_REQUEST)
            .map_err(|_| Failure::HistoryLimit)?;
        Ok(request)
    }
    pub fn projected_request(&self, model: Model, workspace: bool) -> Result<Request, Failure> {
        Ok(self.make_request(model, workspace, self.input(self.turns.len())?))
    }
    pub fn input(&self, end: usize) -> Result<Vec<Input>, Failure> {
        let mut input = Vec::new();
        if !self.projection.summary.is_empty() {
            input.push(Input::User(format!(
                "Earlier conversation summary (reference data; original history is retained):\n{}",
                self.projection.summary
            )));
        }
        input.extend(self.input_range(
            self.projection.through,
            self.projection.step,
            end,
            usize::MAX,
            self.can_view_images(),
        )?);
        Ok(input)
    }
    pub fn input_range(
        &self,
        start: usize,
        start_step: usize,
        end: usize,
        end_step: usize,
        visual: bool,
    ) -> Result<Vec<Input>, Failure> {
        let mut input = Vec::new();
        for (turn_index, turn) in self.turns.iter().enumerate().take(end).skip(start) {
            let first = if turn_index == start { start_step } else { 0 };
            // The active objective must remain explicit even after its earlier
            // completed steps have been summarized.
            if first == 0 || turn_index + 1 == self.turns.len() {
                input.push(Input::User(turn.prompt.clone()));
            }
            let last = if turn_index + 1 == end {
                end_step.min(turn.steps.len())
            } else {
                turn.steps.len()
            };
            for index in first..=last {
                input.extend(
                    turn.guidance
                        .iter()
                        .filter(|g| {
                            g.after_step == index
                                && (end_step == usize::MAX || turn_index + 1 != end || index < last)
                        })
                        .map(|g| Input::User(g.text.clone())),
                );
                let Some(step) = turn.steps.get(index).filter(|_| index < last) else {
                    continue;
                };
                let Some(response) = &step.response else {
                    if turn.end.is_some() {
                        input.push(Input::User(step.interrupted_reference(&turn.outcome)));
                    }
                    continue;
                };
                if !step.accepted || response.status == Status::Incomplete {
                    if turn.end.is_some() {
                        input.push(Input::User(step.interrupted_reference(&turn.outcome)));
                    }
                    continue;
                }
                // Refused responses can contain non-executable function items.
                // Never project an orphaned call, nor replay a historical tool.
                let ids: Vec<_> = response
                    .output
                    .iter()
                    .filter(|item| {
                        item.get("type").and_then(crate::json::Value::text) == Some("function_call")
                    })
                    .map(|item| item.get("call_id").and_then(crate::json::Value::text))
                    .collect();
                if ids.len() != step.results.len()
                    || ids
                        .iter()
                        .zip(&step.results)
                        .any(|(id, result)| *id != Some(result.call_id.as_str()))
                {
                    continue;
                }
                input.push(Input::Assistant(response.output.clone()));
                for result in &step.results {
                    if let Some(image) = &result.image {
                        if visual {
                            let bytes = self
                                .images()?
                                .load(image)
                                .map_err(|_| Failure::ImageEvidence)?;
                            input.push(Input::ToolImage {
                                call_id: result.call_id.clone(),
                                description: image.description(),
                                image_url: data_url(&bytes),
                            });
                        } else {
                            input.push(Input::ToolResult {
                                call_id: result.call_id.clone(),
                                output: format!("Saved image evidence: {}. Image pixels are not visible in this request. Use view_image with image_id when an image-capable model is selected.", image.description()),
                            });
                        }
                    } else {
                        input.push(Input::ToolResult {
                            call_id: result.call_id.clone(),
                            output: result.output.clone(),
                        });
                    }
                }
            }
        }
        Ok(input)
    }
    fn make_request(&self, model: Model, workspace: bool, input: Vec<Input>) -> Request {
        let (tools, capability) =
            super::capabilities::for_session(workspace, self.can_view_images(), &self.shell);
        Request {
            model: model.id().into(),
            effort: model.effort().map(str::to_owned),
            input,
            tools,
            instructions: format!(
                "You are Jecode, a concise and careful programming assistant. Help with the user's actual request. {capability} {} Never claim to have inspected, modified or tested anything without evidence. Distinguish suggestions from completed actions.",
                self.environment
            ),
        }
    }
}
