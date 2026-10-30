use super::terminal::Waker;
use crate::agent::Agent;
use crate::cancel::Cancellation;
use crate::events::Event;
use std::collections::VecDeque;
#[cfg(test)]
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::TryRecvError;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

const EVENT_CAPACITY: usize = 16;

struct EventQueue {
    events: VecDeque<Event>,
    streaming: Option<String>,
    sender_alive: bool,
    receiver_alive: bool,
    /// Wakes the loop when the queue stops being empty, once per drain.
    waker: Option<Waker>,
}

impl EventQueue {
    fn empty(&self) -> bool {
        self.events.is_empty() && self.streaming.is_none()
    }
}

struct EventChannel {
    queue: Mutex<EventQueue>,
    ready: Condvar,
}

struct EventSender(Arc<EventChannel>);

pub struct EventReceiver(Arc<EventChannel>);

fn event_channel() -> (EventSender, EventReceiver) {
    let channel = Arc::new(EventChannel {
        queue: Mutex::new(EventQueue {
            events: VecDeque::new(),
            streaming: None,
            sender_alive: true,
            receiver_alive: true,
            waker: None,
        }),
        ready: Condvar::new(),
    });
    (EventSender(Arc::clone(&channel)), EventReceiver(channel))
}

impl EventSender {
    /// Queues the event; the loop is woken only when the queue was empty, so
    /// a burst of tokens costs one wake until the loop drains it.
    fn send(&self, event: Event, cancellation: &Cancellation) -> Result<(), String> {
        let queue = self.0.queue.lock().unwrap();
        let waker = queue.empty().then(|| queue.waker.clone()).flatten();
        let result = self.queue(queue, event, cancellation);
        if let Some(waker) = waker {
            waker.wake();
        }
        result
    }

    fn queue(
        &self,
        mut queue: std::sync::MutexGuard<'_, EventQueue>,
        event: Event,
        cancellation: &Cancellation,
    ) -> Result<(), String> {
        if !queue.receiver_alive {
            return Err("Terminal output closed".into());
        }
        match event {
            Event::Streaming { text } => {
                // Updates contain the whole response. Keep only its latest state.
                queue.streaming = Some(text);
                self.0.ready.notify_one();
                return Ok(());
            }
            Event::Reasoning | Event::Working if queue.streaming.is_some() => return Ok(()),
            Event::Reasoning | Event::Working if queue.events.len() == EVENT_CAPACITY => {
                return Ok(());
            }
            Event::Message { .. }
            | Event::Preparing
            | Event::Waiting { .. }
            | Event::RequestDiscarded
            | Event::Recovering { .. } => {
                // A completed or abandoned response makes its last preview stale.
                queue.streaming = None;
            }
            _ => {
                if let Some(text) = queue.streaming.take() {
                    queue = self.wait_for_space(queue, cancellation)?;
                    queue.events.push_back(Event::Streaming { text });
                    self.0.ready.notify_one();
                }
            }
        }
        queue = self.wait_for_space(queue, cancellation)?;
        queue.events.push_back(event);
        self.0.ready.notify_one();
        Ok(())
    }

    fn wait_for_space<'a>(
        &self,
        mut queue: std::sync::MutexGuard<'a, EventQueue>,
        cancellation: &Cancellation,
    ) -> Result<std::sync::MutexGuard<'a, EventQueue>, String> {
        while queue.events.len() == EVENT_CAPACITY {
            if !queue.receiver_alive {
                return Err("Terminal output closed".into());
            }
            if cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            queue = self
                .0
                .ready
                .wait_timeout(queue, Duration::from_millis(5))
                .unwrap()
                .0;
        }
        if !queue.receiver_alive {
            return Err("Terminal output closed".into());
        }
        Ok(queue)
    }
}

impl Drop for EventSender {
    fn drop(&mut self) {
        let mut queue = self.0.queue.lock().unwrap();
        queue.sender_alive = false;
        let waker = queue.waker.clone();
        drop(queue);
        self.0.ready.notify_all();
        // The finished turn is collected without waiting for a timer.
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl EventReceiver {
    /// Wakes the loop for events from now on, and at once for any queued.
    pub fn wake_with(&self, waker: Waker) {
        let mut queue = self.0.queue.lock().unwrap();
        let pending = !queue.empty() || !queue.sender_alive;
        queue.waker = Some(waker.clone());
        drop(queue);
        if pending {
            waker.wake();
        }
    }

    /// The worker has sent its last event and is about to finish.
    pub fn closed(&self) -> bool {
        !self.0.queue.lock().unwrap().sender_alive
    }

    pub fn try_recv(&self) -> Result<Event, TryRecvError> {
        let mut queue = self.0.queue.lock().unwrap();
        if let Some(event) = Self::take(&mut queue) {
            self.0.ready.notify_all();
            Ok(event)
        } else if queue.sender_alive {
            Err(TryRecvError::Empty)
        } else {
            Err(TryRecvError::Disconnected)
        }
    }

    #[cfg(test)]
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Event, RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        let mut queue = self.0.queue.lock().unwrap();
        loop {
            if let Some(event) = Self::take(&mut queue) {
                self.0.ready.notify_all();
                return Ok(event);
            }
            if !queue.sender_alive {
                return Err(RecvTimeoutError::Disconnected);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RecvTimeoutError::Timeout);
            }
            queue = self.0.ready.wait_timeout(queue, remaining).unwrap().0;
        }
    }

    fn take(queue: &mut EventQueue) -> Option<Event> {
        queue
            .events
            .pop_front()
            .or_else(|| queue.streaming.take().map(|text| Event::Streaming { text }))
    }
}

impl Drop for EventReceiver {
    fn drop(&mut self) {
        self.0.queue.lock().unwrap().receiver_alive = false;
        self.0.ready.notify_all();
    }
}

pub struct Worker {
    pub events: EventReceiver,
    task: Option<JoinHandle<(Agent, Result<(), String>)>>,
    cancellation: Cancellation,
}
impl Worker {
    pub fn continue_question(mut agent: Agent) -> Self {
        let cancellation = agent.cancellation();
        cancellation.reset();
        let (sender, events) = event_channel();
        let token = cancellation.clone();
        let task = thread::spawn(move || {
            let result = agent
                .continue_question(&mut |event| sender.send(event, &token))
                .map(|_| ());
            (agent, result)
        });
        Self {
            events,
            task: Some(task),
            cancellation,
        }
    }
    pub fn continue_collaboration(mut agent: Agent) -> Self {
        let cancellation = agent.cancellation();
        let (sender, events) = event_channel();
        let token = cancellation.clone();
        let task = thread::spawn(move || {
            let result = agent
                .continue_collaboration(&mut |event| sender.send(event, &token))
                .map(|_| ());
            (agent, result)
        });
        Self {
            events,
            task: Some(task),
            cancellation,
        }
    }
    pub fn start(mut agent: Agent, prompt: crate::attachments::Prompt) -> Self {
        let cancellation = agent.cancellation();
        cancellation.reset();
        let (sender, events) = event_channel();
        let token = cancellation.clone();
        let task = thread::spawn(move || {
            let result = agent.run_turn(&prompt, &mut |event| sender.send(event, &token));
            (agent, result)
        });
        Self {
            events,
            task: Some(task),
            cancellation,
        }
    }
    pub fn finished(&self) -> bool {
        self.task.as_ref().unwrap().is_finished()
    }
    pub fn finish(mut self) -> Result<(Agent, Result<(), String>), String> {
        self.task
            .take()
            .unwrap()
            .join()
            .map_err(|_| "Agent worker failed".into())
    }
    /// Requests cancellation.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            self.cancel();
            let _ = task.join();
        }
    }
}

#[cfg(test)]
mod tests;
