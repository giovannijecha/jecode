use crate::agent::Agent;
use crate::cancel::Cancellation;
use crate::events::Event;
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct Worker {
    pub events: Receiver<Event>,
    task: Option<JoinHandle<(Agent, Result<(), String>)>>,
    cancellation: Cancellation,
}
impl Worker {
    pub fn start(mut agent: Agent, prompt: String) -> Self {
        let cancellation = agent.cancellation();
        cancellation.reset();
        let (sender, events) = mpsc::sync_channel(16);
        let token = cancellation.clone();
        let task = thread::spawn(move || {
            let result = agent.run_turn(&prompt, &mut |event| {
                let mut pending = event;
                loop {
                    match sender.try_send(pending) {
                        Ok(()) => return Ok(()),
                        Err(mpsc::TrySendError::Disconnected(_)) => {
                            return Err("Terminal output closed".into());
                        }
                        Err(mpsc::TrySendError::Full(event)) => {
                            if token.requested() {
                                return Err("Operation cancelled".into());
                            }
                            pending = event;
                            thread::sleep(Duration::from_millis(5));
                        }
                    }
                }
            });
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
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            self.cancellation.cancel();
            let _ = task.join();
        }
    }
}

#[cfg(test)]
mod tests;
