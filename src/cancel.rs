use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
    pub fn requested(&self) -> bool {
        self.0.load(Ordering::Relaxed)
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
}
