//! The root process's own outcome, observed separately from the lifetime of
//! the supervised boundary around it.
//!
//! A command's boundary ends when its whole tree has ended: on Windows when
//! the Job Object is empty, on Unix when the root has exited and no process
//! holds its output open. The root can end long before that, for example a
//! launcher that starts an application or a build whose daemon stays alive.
//! `Watch` carries that earlier fact to observers while supervision continues;
//! it never ends supervision or changes cleanup.

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Supervised processes reported after the root exited, from the native
/// boundary. Windows reads them from the Job Object; Unix has no list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remaining {
    /// Processes the boundary still counts as running.
    pub count: usize,
    /// Process IDs with image names, bounded; may be shorter than `count`.
    pub processes: Vec<(u32, String)>,
}

/// The root process ended while the supervised boundary was still open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootExit {
    pub pid: u32,
    /// None when the platform reports no code, such as a Unix signal.
    pub exit_code: Option<i32>,
    /// From launch until the supervisor observed the root's exit.
    pub elapsed: Duration,
    /// The latest report of what keeps the boundary open; `None` on Unix,
    /// where the boundary waits for the command's output to close.
    pub remaining: Option<Remaining>,
}

type Listener = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Default)]
pub struct Watch(Arc<Inner>);

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    root: Option<RootExit>,
    listener: Option<Listener>,
}

impl Watch {
    pub fn root(&self) -> Option<RootExit> {
        self.0.state.lock().unwrap().root.clone()
    }

    /// Calls `listener` after each change, and at once when the root has
    /// already exited, so registration can never miss the event.
    pub fn on_change(&self, listener: impl Fn() + Send + Sync + 'static) {
        let listener: Listener = Arc::new(listener);
        let mut state = self.0.state.lock().unwrap();
        state.listener = Some(Arc::clone(&listener));
        let known = state.root.is_some();
        drop(state);
        if known {
            listener();
        }
    }

    /// Records the root's exit or a newer report of what remains. The first
    /// report fixes the exit; later ones only refresh `remaining`.
    pub(super) fn record(&self, report: RootExit) {
        let listener = {
            let mut state = self.0.state.lock().unwrap();
            match state.root.as_mut() {
                Some(known) if known.remaining == report.remaining => return,
                Some(known) => known.remaining = report.remaining,
                None => state.root = Some(report),
            }
            state.listener.clone()
        };
        // A callback can read the root or replace its listener without deadlocking.
        if let Some(listener) = listener {
            listener();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn exit(remaining: Option<Remaining>) -> RootExit {
        RootExit {
            pid: 10,
            exit_code: Some(7),
            elapsed: Duration::from_millis(5),
            remaining,
        }
    }

    #[test]
    fn the_first_exit_is_kept_and_later_reports_refresh_only_what_remains() {
        let watch = Watch::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        watch.on_change(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        let one = Remaining {
            count: 1,
            processes: vec![(11, "child.exe".into())],
        };
        watch.record(exit(Some(one.clone())));
        watch.record(exit(Some(one.clone())));
        let mut later = exit(Some(Remaining {
            count: 0,
            processes: Vec::new(),
        }));
        later.exit_code = Some(1);
        later.elapsed = Duration::from_secs(9);
        watch.record(later);
        let root = watch.root().unwrap();
        assert_eq!(root.exit_code, Some(7));
        assert_eq!(root.elapsed, Duration::from_millis(5));
        assert_eq!(root.remaining.unwrap().count, 0);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "an unchanged report is not news"
        );
    }

    #[test]
    fn a_late_listener_still_learns_an_earlier_exit() {
        let watch = Watch::default();
        watch.record(exit(None));
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        watch.on_change(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn registration_racing_with_the_first_exit_cannot_lose_it() {
        for _ in 0..64 {
            let watch = Watch::default();
            let start = Arc::new(Barrier::new(3));
            let calls = Arc::new(AtomicUsize::new(0));
            let registering = {
                let watch = watch.clone();
                let start = Arc::clone(&start);
                let calls = Arc::clone(&calls);
                std::thread::spawn(move || {
                    start.wait();
                    watch.on_change(move || {
                        calls.fetch_add(1, Ordering::SeqCst);
                    });
                })
            };
            let recording = {
                let watch = watch.clone();
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    watch.record(exit(None));
                })
            };
            start.wait();
            registering.join().unwrap();
            recording.join().unwrap();
            assert!(watch.root().is_some());
            assert!(calls.load(Ordering::SeqCst) >= 1);
        }
    }

    #[test]
    fn listener_can_read_and_update_the_watch_without_holding_its_lock() {
        let watch = Watch::default();
        let observing = watch.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        watch.on_change(move || {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                assert!(observing.root().is_some());
                observing.record(exit(Some(Remaining {
                    count: 1,
                    processes: vec![(11, "child.exe".into())],
                })));
            }
        });
        watch.record(exit(None));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
