//! One autosave thread per session, started by its first background save.
//! It holds the session weakly between saves, so closing the conversation
//! still drops it and releases its lease.

use super::{Handle, Shared};
use std::sync::{Arc, Condvar, Mutex, Weak, atomic::Ordering};

#[derive(Default)]
pub(super) struct Autosave {
    state: Mutex<State>,
    ready: Condvar,
}

#[derive(Default)]
struct State {
    started: bool,
    requested: bool,
    stopped: bool,
    #[cfg(test)]
    threads: usize,
}

impl Autosave {
    /// Asks for one save of `shared`, starting the thread on first use.
    pub fn request(self: &Arc<Self>, shared: &Arc<Shared>) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if !state.started {
            let (autosave, shared) = (Arc::clone(self), Arc::downgrade(shared));
            std::thread::Builder::new()
                .name("jecode-autosave".into())
                .spawn(move || run(&autosave, &shared))
                .map_err(|error| format!("Could not start autosave: {error}"))?;
            state.started = true;
            #[cfg(test)]
            {
                state.threads += 1;
            }
        }
        state.requested = true;
        self.ready.notify_one();
        Ok(())
    }

    pub fn stop(&self) {
        self.state.lock().unwrap().stopped = true;
        self.ready.notify_all();
    }

    #[cfg(test)]
    pub fn threads(&self) -> usize {
        self.state.lock().unwrap().threads
    }
}

fn run(autosave: &Autosave, shared: &Weak<Shared>) {
    loop {
        {
            let mut state = autosave.state.lock().unwrap();
            while !state.requested && !state.stopped {
                state = autosave.ready.wait(state).unwrap();
            }
            if state.stopped {
                return;
            }
            state.requested = false;
        }
        let Some(shared) = shared.upgrade() else {
            return;
        };
        let handle = Handle(shared);
        let result = handle.flush();
        *handle.0.background_result.lock().unwrap() = Some(result);
        handle.0.background_save.store(false, Ordering::Release);
    }
}
