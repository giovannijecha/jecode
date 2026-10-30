//! Lock order between the main conversation and collaborator receipts.
use super::*;
use crate::{
    attachments::{Inputs, Pool},
    backend::{Backend, Completion, Failure, Limits, Services, Update, openrouter::OpenRouter},
    cancel::Cancellation,
    effort::Effort,
    redact::Redactor,
    test_support::Directory,
};
use std::sync::{Arc, Condvar, Mutex, mpsc};

/// Pauses the next armed context estimate while it holds the messages lock.
#[derive(Default)]
struct Gate {
    state: Mutex<(bool, bool, bool)>,
    changed: Condvar,
}

impl Gate {
    fn arm(&self) {
        self.state.lock().unwrap().0 = true;
    }

    fn pass(&self) {
        let mut state = self.state.lock().unwrap();
        if !std::mem::take(&mut state.0) {
            return;
        }
        state.1 = true;
        self.changed.notify_all();
        while !state.2 {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn wait_entered(&self) {
        let (state, _) = self
            .changed
            .wait_timeout_while(
                self.state.lock().unwrap(),
                Duration::from_secs(10),
                |state| !state.1,
            )
            .unwrap();
        assert!(
            state.1,
            "context estimate did not reach the guarded messages lock"
        );
    }

    fn release(&self) {
        self.state.lock().unwrap().2 = true;
        self.changed.notify_all();
    }
}

struct Paused {
    inner: Box<dyn Backend>,
    gate: Arc<Gate>,
}

impl Backend for Paused {
    fn id(&self) -> &'static str {
        self.inner.id()
    }
    fn model(&self) -> &str {
        self.inner.model()
    }
    fn effort(&self) -> Effort {
        self.inner.effort()
    }
    fn set_effort(&mut self, effort: Effort) {
        self.inner.set_effort(effort);
    }
    fn inputs(&self) -> Inputs {
        self.gate.pass();
        self.inner.inputs()
    }
    fn limits(&mut self) -> Result<Option<Limits>, Failure> {
        self.inner.limits()
    }
    fn known_limits(&self) -> Option<Limits> {
        self.inner.known_limits()
    }
    fn sample(
        &self,
        messages: &[Value],
        with_tools: bool,
        output_tokens: Option<usize>,
        pool: Option<&Pool>,
        emit: &mut dyn FnMut(Update) -> Result<(), String>,
    ) -> Result<Completion, Failure> {
        self.inner
            .sample(messages, with_tools, output_tokens, pool, emit)
    }
    fn redactor(&self) -> Redactor {
        self.inner.redactor()
    }
    fn set_cancellation(&mut self, cancellation: Cancellation) {
        self.inner.set_cancellation(cancellation);
    }
    fn set_conversation(&mut self, id: Option<String>) {
        self.inner.set_conversation(id);
    }
    fn services(&self) -> Services {
        self.inner.services()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self.inner.as_any()
    }
}

#[test]
fn receipt_during_context_estimate_does_not_deadlock() {
    let project = Directory::new();
    let home = Directory::new();
    let gate = Arc::new(Gate::default());
    let mut agent = Agent::with_backend(
        Box::new(Paused {
            inner: Box::new(OpenRouter::fixture("http://127.0.0.1:9".into())),
            gate: Arc::clone(&gate),
        }),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let observer = agent.collaboration();
    let (done, finished) = mpsc::channel();
    gate.arm();
    let estimate = {
        let done = done.clone();
        std::thread::spawn(move || {
            agent.locked_estimate();
            let _ = done.send("estimate");
            agent
        })
    };
    // The estimate now holds the conversation messages. A receipt takes
    // collaboration state, then waits for those messages.
    gate.wait_entered();
    let receipt = std::thread::spawn(move || {
        let saved = observer.receipt("sent", None, 0, "Agents", vec![]);
        let _ = done.send("receipt");
        saved
    });
    std::thread::sleep(Duration::from_millis(200));
    gate.release();
    for _ in 0..2 {
        finished
            .recv_timeout(Duration::from_secs(10))
            .expect("context estimate and collaborator receipt deadlocked");
    }
    receipt.join().unwrap().unwrap();
    drop(estimate.join().unwrap());
}
