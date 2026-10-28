//! A conversation-owned question channel. Answers are journaled before the
//! worker can observe them, so a resumed call keeps its original tool ID.

use crate::{cancel::Cancellation, export::Archive, json::Value, sessions::Handle};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

mod persistence;
pub(crate) use persistence::{recovery, validate_answer};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub description: String,
    pub recommended: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub title: String,
    pub question: String,
    pub context: String,
    pub options: Vec<Choice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub question: Question,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub id: String,
    pub text: String,
    pub cursor: usize,
    pub choice: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    Answer {
        choice: Option<String>,
        text: String,
    },
    Dismissed,
}

impl Reply {
    pub(crate) fn result(&self) -> Value {
        match self {
            Self::Answer { choice, text } => Value::object([
                ("outcome", Value::string("answered")),
                ("choice", choice.as_ref().map_or(Value::Null, Value::string)),
                ("text", Value::string(text)),
            ]),
            Self::Dismissed => Value::object([("outcome", Value::string("dismissed"))]),
        }
    }
}

impl Draft {
    pub(crate) fn value(&self) -> Value {
        Value::object([
            ("id", Value::string(&self.id)),
            ("text", Value::string(&self.text)),
            ("cursor", Value::number(self.cursor)),
            (
                "choice",
                self.choice.as_ref().map_or(Value::Null, Value::string),
            ),
        ])
    }

    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or("Invalid question draft ID")?;
        let text = value
            .get("text")
            .and_then(Value::as_str)
            .ok_or("Invalid question draft text")?;
        let cursor = value
            .get("cursor")
            .and_then(Value::as_usize)
            .ok_or("Invalid question draft cursor")?;
        if id.is_empty()
            || text.len() > 64 * 1024
            || cursor > text.len()
            || !text.is_char_boundary(cursor)
        {
            return Err("Invalid question draft".into());
        }
        let choice = match value.get("choice") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            _ => return Err("Invalid question draft choice".into()),
        };
        Ok(Self {
            id: id.into(),
            text: text.into(),
            cursor,
            choice,
        })
    }
}

#[derive(Default)]
struct State {
    available: bool,
    suspended: bool,
    request: Option<Request>,
    reply: Option<Reply>,
    archive: Option<Archive>,
    handle: Option<Handle>,
}

#[derive(Clone, Default)]
pub struct Observer(Arc<(Mutex<State>, Condvar)>);

impl Observer {
    /// Closing an input surface preserves an unanswered question for resume.
    /// Explicit interruption uses Cancellation instead.
    pub fn suspend(&self) {
        let mut state = self.0.0.lock().unwrap();
        state.suspended = state.request.is_some() && state.reply.is_none();
        state.available = false;
        self.0.1.notify_all();
    }

    pub(crate) fn is_suspended(&self) -> bool {
        let state = self.0.0.lock().unwrap();
        state.suspended && state.request.is_some() && state.reply.is_none()
    }
    pub fn set_available(&self, available: bool) {
        let (state, ready) = &*self.0;
        state.lock().unwrap().available = available;
        ready.notify_all();
    }

    pub fn pending(&self) -> Option<Request> {
        let state = self.0.0.lock().unwrap();
        state
            .reply
            .is_none()
            .then(|| state.request.clone())
            .flatten()
    }

    pub(crate) fn current(&self) -> Option<Request> {
        self.0.0.lock().unwrap().request.clone()
    }

    pub fn answer(&self, id: &str, choice: Option<String>, text: String) -> Result<(), String> {
        if text.len() > 64 * 1024 {
            return Err("Answer exceeds 64 KiB".into());
        }
        let mut state = self.0.0.lock().unwrap();
        let request = state.request.as_ref().ok_or("No question is waiting")?;
        if request.id != id || state.reply.is_some() {
            return Err("This question is no longer waiting".into());
        }
        if let Some(choice) = &choice
            && !request
                .question
                .options
                .iter()
                .any(|option| &option.id == choice)
        {
            return Err("Unknown answer choice".into());
        }
        if choice.is_none() && text.trim().is_empty() {
            return Err("Enter an answer or choose an option".into());
        }
        let reply = Reply::Answer { choice, text };
        self.save_reply(&state, &reply)?;
        state.reply = Some(reply);
        self.0.1.notify_all();
        Ok(())
    }

    pub fn dismiss(&self, id: &str) -> Result<(), String> {
        let mut state = self.0.0.lock().unwrap();
        if state.request.as_ref().map(|request| request.id.as_str()) != Some(id)
            || state.reply.is_some()
        {
            return Err("This question is no longer waiting".into());
        }
        let reply = Reply::Dismissed;
        self.save_reply(&state, &reply)?;
        state.reply = Some(reply);
        self.0.1.notify_all();
        Ok(())
    }

    fn save_reply(&self, state: &State, reply: &Reply) -> Result<(), String> {
        let request = state.request.as_ref().unwrap();
        let event = Value::object([
            ("type", Value::string("user_answer")),
            ("tool_call_id", Value::string(&request.id)),
            ("result", reply.result()),
        ]);
        state
            .handle
            .as_ref()
            .ok_or("Question session is unavailable")?
            .append_event(
                state
                    .archive
                    .as_ref()
                    .ok_or("Question archive is unavailable")?,
                event,
            )?;
        Ok(())
    }

    pub(crate) fn bind(&self, archive: Archive, handle: Option<Handle>) {
        let mut state = self.0.0.lock().unwrap();
        state.archive = Some(archive);
        state.handle = handle;
        state.request = None;
        state.reply = None;
        state.suspended = false;
        if let Some(handle) = &state.handle {
            let doc = handle.snapshot();
            if let Some((request, reply)) = recovery(&doc.messages, &doc.events) {
                state.request = Some(request);
                state.reply = reply;
            }
        }
        self.0.1.notify_all();
    }

    pub(crate) fn begin(&self, request: Request) -> Result<bool, String> {
        let mut state = self.0.0.lock().unwrap();
        if !state.available || state.handle.is_none() {
            return Ok(false);
        }
        if state.request.is_some() {
            return Err("Another question is already waiting".into());
        }
        let event = Value::object([
            ("type", Value::string("user_question")),
            ("tool_call_id", Value::string(&request.id)),
            ("question", request.question.value()),
        ]);
        state
            .handle
            .as_ref()
            .unwrap()
            .append_event(state.archive.as_ref().unwrap(), event)?;
        state.request = Some(request);
        state.reply = None;
        state.suspended = false;
        Ok(true)
    }

    pub(crate) fn wait(&self, cancellation: &Cancellation) -> Option<Reply> {
        let mut state = self.0.0.lock().unwrap();
        loop {
            if let Some(reply) = &state.reply {
                return Some(reply.clone());
            }
            if cancellation.requested() || !state.available {
                return None;
            }
            state = self
                .0
                .1
                .wait_timeout(state, Duration::from_millis(100))
                .unwrap()
                .0;
        }
    }

    pub(crate) fn complete(&self, id: &str) {
        let mut state = self.0.0.lock().unwrap();
        if state
            .request
            .as_ref()
            .is_some_and(|request| request.id == id)
        {
            state.request = None;
            state.reply = None;
        }
    }

    pub(crate) fn saved_reply(&self) -> Option<Reply> {
        self.0.0.lock().unwrap().reply.clone()
    }
}

impl Question {
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        let get = |key| value.get(key).and_then(Value::as_str);
        let title = get("title").ok_or("title must be a string")?;
        let question = get("question").ok_or("question must be a string")?;
        let context = optional_text(value, "context")?;
        if title.trim().is_empty()
            || title.len() > 160
            || question.trim().is_empty()
            || question.len() > 4096
            || context.len() > 4096
        {
            return Err("Question title, text or context is invalid".into());
        }
        let options = match value.get("options") {
            None => &[][..],
            Some(Value::Array(options)) if options.len() <= 6 => options.as_slice(),
            _ => return Err("options must contain at most six choices".into()),
        };
        let mut parsed = Vec::new();
        for option in options {
            let id = option
                .get("id")
                .and_then(Value::as_str)
                .ok_or("option id must be a string")?;
            let label = option
                .get("label")
                .and_then(Value::as_str)
                .ok_or("option label must be a string")?;
            let description = optional_text(option, "description")?;
            let recommended = match option.get("recommended") {
                None | Some(Value::Bool(false)) => false,
                Some(Value::Bool(true)) => true,
                _ => return Err("recommended must be a boolean".into()),
            };
            if id.is_empty()
                || id.len() > 80
                || label.trim().is_empty()
                || label.len() > 160
                || description.len() > 512
                || parsed.iter().any(|item: &Choice| item.id == id)
            {
                return Err("Question option is invalid or duplicated".into());
            }
            parsed.push(Choice {
                id: id.into(),
                label: label.into(),
                description: description.into(),
                recommended,
            });
        }
        Ok(Self {
            title: title.into(),
            question: question.into(),
            context: context.into(),
            options: parsed,
        })
    }

    pub(crate) fn value(&self) -> Value {
        Value::object([
            ("title", Value::string(&self.title)),
            ("question", Value::string(&self.question)),
            ("context", Value::string(&self.context)),
            (
                "options",
                Value::Array(
                    self.options
                        .iter()
                        .map(|item| {
                            Value::object([
                                ("id", Value::string(&item.id)),
                                ("label", Value::string(&item.label)),
                                ("description", Value::string(&item.description)),
                                ("recommended", Value::Bool(item.recommended)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

fn optional_text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    match value.get(key) {
        None => Ok(""),
        Some(Value::String(text)) => Ok(text),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

impl Reply {
    fn parse(value: &Value) -> Option<Self> {
        match value.get("outcome")?.as_str()? {
            "answered" => Some(Self::Answer {
                choice: match value.get("choice")? {
                    Value::Null => None,
                    Value::String(value) => Some(value.clone()),
                    _ => return None,
                },
                text: value.get("text")?.as_str()?.into(),
            }),
            "dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}
