//! A conversation-owned question channel. Answers are journaled before the
//! worker can observe them, so a resumed call keeps its original tool ID.

use crate::{cancel::Cancellation, export::Archive, json::Value, sessions::Handle};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

mod draft;
mod persistence;
mod question;
mod reply;
pub use draft::{AnswerDraft, Draft};
pub(crate) use persistence::{recovery, validate_answer};
pub use question::{LEGACY_ID, MAX_OPTIONS, MAX_QUESTIONS, Question, Request};
pub(crate) use reply::Reply;
pub use reply::{Answer, TEXT_LIMIT};

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

    /// One answer per question, in order.
    pub fn answer(&self, id: &str, answers: Vec<Answer>) -> Result<(), String> {
        let mut state = self.0.0.lock().unwrap();
        let request = state.request.as_ref().ok_or("No question is waiting")?;
        if request.id != id || state.reply.is_some() {
            return Err("This question is no longer waiting".into());
        }
        request.check(&answers)?;
        let reply = Reply::Answered(answers);
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
        let Value::Object(mut event) = request.arguments() else {
            unreachable!()
        };
        event.insert("type".into(), Value::string("user_question"));
        event.insert("tool_call_id".into(), Value::string(&request.id));
        state
            .handle
            .as_ref()
            .unwrap()
            .append_event(state.archive.as_ref().unwrap(), Value::Object(event))?;
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

#[cfg(test)]
mod tests;
