//! The composer state a session keeps: draft, queued prompts, staged
//! attachments and an unanswered question's answers.

use super::{integer, prompt_list, prompts, required, string};
use crate::attachments::{Attachment, Prompt};
use crate::{json::Value, redact::Redactor};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub text: String,
    pub cursor: usize,
    /// One per attachment marker in `text`, in order.
    pub attachments: Vec<Attachment>,
}

impl Draft {
    pub fn prompt(&self) -> Prompt {
        Prompt::new(self.text.clone(), self.attachments.clone())
    }

    pub fn from_prompt(prompt: Prompt) -> Self {
        Self {
            cursor: prompt.text.len(),
            text: prompt.text,
            attachments: prompt.attachments,
        }
    }

    pub(super) fn value(&self) -> Value {
        let Value::Object(mut fields) = self.prompt().value() else {
            // Journal deltas merge object fields, so clearing a list must be explicit.
            return Value::object([
                ("text", Value::string(&self.text)),
                ("cursor", Value::number(self.cursor)),
                ("attachments", Value::Array(vec![])),
            ]);
        };
        fields.insert("cursor".into(), Value::number(self.cursor));
        Value::Object(fields)
    }

    pub(super) fn parse(value: &Value) -> Result<Self, String> {
        let text = string(value, "text")?.to_string();
        let cursor =
            usize::try_from(integer(value, "cursor")?).map_err(|_| "Invalid saved cursor")?;
        if !text.is_char_boundary(cursor) {
            return Err("Invalid saved draft or cursor".into());
        }
        let prompt = Prompt::parse(value)?;
        let cursor = if prompt.text == text {
            cursor
        } else {
            prompt.text.len()
        };
        Ok(Self {
            text: prompt.text,
            cursor,
            attachments: prompt.attachments,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub draft: Draft,
    pub question: Option<crate::ask::Draft>,
    /// Attachments staged by the line-based chat for its next message.
    pub staged: Vec<Attachment>,
    pub queued: Vec<Prompt>,
    pub paused: Vec<Draft>,
    pub previous: Option<Draft>,
    pub history: Vec<Prompt>,
}

impl Input {
    pub(crate) fn attachment_ids(&self) -> BTreeSet<String> {
        std::iter::once(&self.draft)
            .chain(&self.paused)
            .chain(self.previous.iter())
            .flat_map(|draft| &draft.attachments)
            .chain(
                self.queued
                    .iter()
                    .chain(&self.history)
                    .flat_map(|prompt| &prompt.attachments),
            )
            .chain(&self.staged)
            .map(|attachment| attachment.id.clone())
            .collect()
    }

    pub fn recover(&mut self) {
        if !self.queued.is_empty() {
            let mut queued: Vec<_> = self.queued.drain(..).map(Draft::from_prompt).collect();
            queued.append(&mut self.paused);
            self.paused = queued;
        }
        if let Some(previous) = self.previous.take() {
            let edited = std::mem::replace(&mut self.draft, previous);
            if !edited.text.is_empty() {
                self.paused.push(edited);
            }
        }
        self.history.retain(|prompt| {
            !prompt.attachments.is_empty() || !prompt.text.trim_start().starts_with('/')
        });
    }

    #[cfg(test)]
    pub fn meaningful(&self) -> bool {
        !self.draft.text.is_empty()
            || !self.staged.is_empty()
            || !self.queued.is_empty()
            || !self.paused.is_empty()
            || self.previous.is_some()
    }

    pub(in crate::sessions) fn redacted(&self, redactor: &Redactor) -> Self {
        let draft = |draft: &Draft| {
            let (text, cursor) = redactor.text_cursor(&draft.text, draft.cursor);
            let prompt = Prompt::new(text, draft.attachments.clone());
            Draft {
                cursor: if prompt.text.is_char_boundary(cursor) && cursor <= prompt.text.len() {
                    cursor
                } else {
                    prompt.text.len()
                },
                text: prompt.text,
                attachments: prompt.attachments,
            }
        };
        let prompt =
            |prompt: &Prompt| Prompt::new(redactor.text(&prompt.text), prompt.attachments.clone());
        Self {
            draft: draft(&self.draft),
            question: self
                .question
                .as_ref()
                .map(|question| question.redacted(redactor)),
            staged: self.staged.clone(),
            queued: self.queued.iter().map(prompt).collect(),
            paused: self.paused.iter().map(draft).collect(),
            previous: self.previous.as_ref().map(draft),
            history: self.history.iter().map(prompt).collect(),
        }
    }

    pub(super) fn value(&self) -> Value {
        Value::object([
            ("draft", self.draft.value()),
            (
                "question",
                self.question
                    .as_ref()
                    .map_or(Value::Null, crate::ask::Draft::value),
            ),
            (
                "staged",
                Value::Array(self.staged.iter().map(Attachment::value).collect()),
            ),
            ("queued", prompts(&self.queued)),
            (
                "paused",
                Value::Array(self.paused.iter().map(Draft::value).collect()),
            ),
            (
                "previous",
                self.previous.as_ref().map_or(Value::Null, Draft::value),
            ),
            ("history", prompts(&self.history)),
        ])
    }

    pub(super) fn parse(value: &Value) -> Result<Self, String> {
        let queued = prompt_list(value, "queued", 8)?;
        let paused = match value.get("paused") {
            None => Vec::new(),
            Some(Value::Array(values)) => {
                values.iter().map(Draft::parse).collect::<Result<_, _>>()?
            }
            Some(_) => return Err("Invalid saved paused drafts".into()),
        };
        let history = prompt_list(value, "history", 50)?;
        Ok(Self {
            draft: Draft::parse(required(value, "draft")?)?,
            question: match value.get("question") {
                None | Some(Value::Null) => None,
                Some(value) => Some(crate::ask::Draft::parse(value)?),
            },
            staged: match value.get("staged") {
                None => Vec::new(),
                Some(Value::Array(values)) => values
                    .iter()
                    .map(Attachment::parse)
                    .collect::<Result<Vec<_>, _>>()?,
                Some(_) => return Err("Invalid saved staged attachments".into()),
            },
            queued,
            paused,
            history,
            previous: match required(value, "previous")? {
                Value::Null => None,
                value => Some(Draft::parse(value)?),
            },
        })
    }
}
