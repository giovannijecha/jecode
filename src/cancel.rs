use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct State {
    requested: AtomicBool,
    parent: Option<Cancellation>,
}

#[derive(Clone, Default)]
pub struct Cancellation(Arc<State>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.requested.store(true, Ordering::Relaxed);
    }
    pub fn reset(&self) {
        self.0.requested.store(false, Ordering::Relaxed);
    }
    pub fn requested(&self) -> bool {
        if self.0.parent.as_ref().is_some_and(Self::requested) {
            self.cancel();
        }
        self.0.requested.load(Ordering::Relaxed)
    }

    /// Follows this operation's cancellation and can also stop independently.
    pub fn child(&self) -> Self {
        Self(Arc::new(State {
            requested: AtomicBool::new(false),
            parent: Some(self.clone()),
        }))
    }

    pub fn wait(&self, delay: std::time::Duration) -> Result<(), String> {
        let started = std::time::Instant::now();
        while started.elapsed() < delay {
            if self.requested() {
                return Err("Operation cancelled".into());
            }
            std::thread::sleep(
                (delay - started.elapsed().min(delay)).min(std::time::Duration::from_millis(20)),
            );
        }
        if self.requested() {
            Err("Operation cancelled".into())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cancellation;

    #[test]
    fn reset_clears_a_shared_cancellation_request() {
        let cancellation = Cancellation::default();
        let other = cancellation.clone();
        other.cancel();
        assert!(cancellation.requested());
        cancellation.reset();
        assert!(!other.requested());
    }

    #[test]
    fn a_child_can_stop_without_cancelling_its_parent() {
        let parent = Cancellation::default();
        let child = parent.child();
        child.cancel();
        assert!(child.requested());
        assert!(!parent.requested());
        assert!(!parent.child().requested());
    }

    #[test]
    fn a_child_retains_observed_parent_cancellation_after_parent_reset() {
        let parent = Cancellation::default();
        let child = parent.child();
        parent.cancel();
        assert!(child.requested());
        parent.reset();
        assert!(child.requested());
        assert!(!parent.child().requested());
    }
}
