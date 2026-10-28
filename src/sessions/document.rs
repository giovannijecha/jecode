use super::{canonical, identifier, now, valid_id};
use crate::attachments::{Attachment, Prompt};
use crate::backend::{Routing, validate_model};
use crate::{effort::Effort, json::Value, redact::Redactor};
use std::collections::BTreeSet;
use std::path::PathBuf;

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

    fn value(&self) -> Value {
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

    fn parse(value: &Value) -> Result<Self, String> {
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

    pub(super) fn redacted(&self, redactor: &Redactor) -> Self {
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

    fn value(&self) -> Value {
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

    fn parse(value: &Value) -> Result<Self, String> {
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

#[derive(Clone, Default)]
pub struct Pending {
    pub active: bool,
    pub tool: Option<String>,
    /// Legacy journal field for parallel calls. Explicit batch IDs mirror here
    /// so older readers still treat interrupted modifying calls as unknown.
    pub reads: Vec<String>,
    /// Calls launched together. Results may complete before earlier calls.
    pub batch: Vec<String>,
    /// Durable completed calls awaiting source-order transcript insertion.
    pub completed: Vec<Value>,
    pub partial: String,
}

#[derive(Clone)]
pub struct Document {
    pub id: String,
    pub created: u64,
    pub updated: u64,
    pub directory: PathBuf,
    pub model: Option<String>,
    pub backend: String,
    pub legacy_origin: Option<crate::conversation::Origin>,
    pub effort: Effort,
    pub compatible_from: usize,
    pub messages: Vec<Value>,
    pub events: Vec<Value>,
    pub pending: Pending,
    pub input: Input,
    pub context: crate::context::Context,
    pub routing: Routing,
}

impl Document {
    pub fn new(directory: PathBuf, model: String, effort: Effort) -> Result<Self, String> {
        let directory = canonical(&directory)?;
        let timestamp = now();
        Ok(Self {
            id: identifier(),
            created: timestamp,
            updated: timestamp,
            directory,
            model: (!model.is_empty()).then_some(model),
            backend: "openrouter".into(),
            legacy_origin: None,
            effort,
            compatible_from: 0,
            messages: vec![],
            events: vec![],
            pending: Pending::default(),
            input: Input::default(),
            context: crate::context::Context::default(),
            routing: Routing::Auto,
        })
    }

    pub fn meaningful(&self) -> bool {
        self.messages
            .iter()
            .any(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            || self
                .events
                .iter()
                .any(|event| event.get("type").and_then(Value::as_str) == Some("collaboration"))
            || !self.input.attachment_ids().is_empty()
    }

    pub fn model_label(&self) -> &str {
        model_label(self.model.as_deref())
    }

    pub fn title(&self) -> String {
        let draft = self.input.draft.prompt().display();
        let text = self
            .messages
            .iter()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .unwrap_or(&draft);
        let title = text
            .split_whitespace()
            .flat_map(|word| word.chars().chain(std::iter::once(' ')))
            .take(100)
            .collect::<String>();
        let title = title.trim_end();
        if title.is_empty() {
            "Untitled conversation".into()
        } else {
            title.into()
        }
    }

    #[cfg(test)]
    pub fn value(&self) -> Value {
        let Value::Object(mut state) = self.state_value() else {
            unreachable!()
        };
        state.insert("messages".into(), Value::Array(self.messages.clone()));
        state.insert("events".into(), Value::Array(self.events.clone()));
        Value::Object(state)
    }

    pub(super) fn state_value(&self) -> Value {
        Value::object([
            ("format", Value::string("jecode.session")),
            ("format_version", Value::number(1)),
            ("jecode_version", Value::string(env!("CARGO_PKG_VERSION"))),
            ("id", Value::string(&self.id)),
            ("created_at_unix_ms", Value::number(self.created)),
            ("updated_at_unix_ms", Value::number(self.updated)),
            (
                "working_directory",
                Value::string(self.directory.to_string_lossy()),
            ),
            (
                "model",
                self.model.as_ref().map_or(Value::Null, Value::string),
            ),
            ("backend", Value::string(&self.backend)),
            (
                "legacy_origin",
                self.legacy_origin
                    .as_ref()
                    .map_or(Value::Null, crate::conversation::Origin::value),
            ),
            ("effort", Value::string(self.effort.name())),
            ("compatible_from", Value::number(self.compatible_from)),
            (
                "pending",
                Value::object([
                    ("active", Value::Bool(self.pending.active)),
                    (
                        "tool",
                        self.pending
                            .tool
                            .as_ref()
                            .map_or(Value::Null, Value::string),
                    ),
                    (
                        "reads",
                        Value::Array(self.pending.reads.iter().map(Value::string).collect()),
                    ),
                    (
                        "batch",
                        Value::Array(self.pending.batch.iter().map(Value::string).collect()),
                    ),
                    ("completed", Value::Array(self.pending.completed.clone())),
                    ("partial_text", Value::string(&self.pending.partial)),
                ]),
            ),
            ("input", self.input.value()),
            ("context", self.context.value()),
            ("routing", self.routing.value()),
        ])
    }

    #[cfg(test)]
    pub fn parse(value: &Value) -> Result<Self, String> {
        let document = Self::parse_state(
            value,
            array(value, "messages")?.to_vec(),
            array(value, "events")?.to_vec(),
        )?;
        super::context::validate(&document)?;
        Ok(document)
    }

    pub(super) fn parse_state(
        value: &Value,
        messages: Vec<Value>,
        events: Vec<Value>,
    ) -> Result<Self, String> {
        if string(value, "format")? != "jecode.session" || integer(value, "format_version")? != 1 {
            return Err("Unsupported session format or version".into());
        }
        let id = string(value, "id")?.to_string();
        if !valid_id(&id) {
            return Err("Invalid session identifier".into());
        }
        let directory = PathBuf::from(string(value, "working_directory")?);
        if !directory.is_absolute() {
            return Err("Session directory must be absolute".into());
        }
        let model = parse_model(value)?;
        let backend = match value.get("backend") {
            None => "openrouter".to_owned(),
            Some(Value::String(id)) if !id.is_empty() && !id.chars().any(char::is_control) => {
                id.clone()
            }
            _ => return Err("Invalid saved backend identifier".into()),
        };
        crate::backend::validate_id(&backend)?;
        let legacy_origin = match value.get("legacy_origin") {
            None => model
                .as_ref()
                .map(|model| crate::conversation::Origin::new(&backend, model)),
            Some(Value::Null) => None,
            Some(value) => Some(crate::conversation::Origin::parse(value)?),
        };
        let pending = required(value, "pending")?;
        let active = match required(pending, "active")? {
            Value::Bool(active) => *active,
            _ => return Err("Invalid pending turn state".into()),
        };
        let mut context = crate::context::Context::parse(value.get("context"))?;
        context.restore_returned(&events, messages.len());
        let document = Self {
            id,
            directory,
            model,
            backend,
            legacy_origin,
            created: integer(value, "created_at_unix_ms")?,
            updated: integer(value, "updated_at_unix_ms")?,
            effort: Effort::parse(string(value, "effort")?)?,
            compatible_from: usize::try_from(integer(value, "compatible_from")?)
                .map_err(|_| "Invalid model compatibility boundary")?,
            messages,
            events,
            pending: Pending {
                active,
                tool: match required(pending, "tool")? {
                    Value::Null => None,
                    Value::String(id) if !id.is_empty() => Some(id.clone()),
                    _ => return Err("Invalid pending tool".into()),
                },
                reads: pending_ids(pending, "reads")?,
                batch: pending_ids(pending, "batch")?,
                completed: match pending.get("completed") {
                    None => Vec::new(),
                    Some(Value::Array(completed)) => completed.clone(),
                    _ => return Err("Invalid pending completed calls".into()),
                },
                partial: string(pending, "partial_text")?.into(),
            },
            input: Input::parse(required(value, "input")?)?,
            context,
            // Sessions saved before routing existed, or with a mode this
            // version does not know, keep sending no routing preferences.
            routing: value
                .get("routing")
                .and_then(Routing::parse)
                .unwrap_or(Routing::Default),
        };
        Ok(document)
    }
}

fn pending_ids(pending: &Value, key: &str) -> Result<Vec<String>, String> {
    match pending.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(ids)) => ids
            .iter()
            .map(|id| {
                id.as_str()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| format!("Invalid pending {key} call"))
            })
            .collect(),
        _ => Err(format!("Invalid pending {key}")),
    }
}

pub(super) fn model_label(model: Option<&str>) -> &str {
    model.unwrap_or("Not selected")
}

pub(super) fn parse_model(value: &Value) -> Result<Option<String>, String> {
    match required(value, "model")? {
        Value::Null => Ok(None),
        Value::String(model) => {
            validate_model(model)?;
            Ok(Some(model.clone()))
        }
        _ => Err("Invalid saved model selection".into()),
    }
}

fn prompts(values: &[Prompt]) -> Value {
    Value::Array(values.iter().map(Prompt::value).collect())
}

pub(super) fn required<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value
        .get(key)
        .ok_or_else(|| format!("Session is missing {key}"))
}
pub(super) fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    required(value, key)?
        .as_str()
        .ok_or_else(|| format!("Invalid session {key}"))
}
pub(super) fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], String> {
    required(value, key)?
        .as_array()
        .ok_or_else(|| format!("Invalid session {key}"))
}
pub(super) fn integer(value: &Value, key: &str) -> Result<u64, String> {
    match required(value, key)? {
        Value::Number(number) => number.parse().map_err(|_| format!("Invalid session {key}")),
        _ => Err(format!("Invalid session {key}")),
    }
}
fn prompt_list(value: &Value, key: &str, limit: usize) -> Result<Vec<Prompt>, String> {
    let values = array(value, key)?;
    if values.len() > limit {
        return Err(format!("Too many saved {key}"));
    }
    values
        .iter()
        .map(|value| Prompt::parse(value).map_err(|_| format!("Invalid saved {key}")))
        .collect()
}
