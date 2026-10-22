//! One process's shared archive handle. Tools acquire path-specific recovery
//! synchronously; the worker maintains retention and explicit notifications.
use super::identity::{self, Location};
use super::locks::Leases;
use super::record::State;
use super::scan::{self, Summary};
use super::store::Store;
use crate::cancel::Cancellation;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

mod readiness;

const TICK: Duration = Duration::from_millis(500);
const DEBOUNCE: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Watch the workspace with the platform helper where one exists.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub watch: bool,
    /// Run archive maintenance and deferred completion on a background thread.
    pub background: bool,
    /// Verify recorded paths when an explicit rescan is requested.
    pub verify: bool,
}

impl Options {
    pub fn interactive() -> Self {
        Self {
            watch: cfg!(windows),
            background: true,
            verify: cfg!(windows),
        }
    }

    pub fn offline() -> Self {
        Self {
            watch: false,
            background: false,
            verify: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// The first capture of the scope, with paths captured so far.
    Baseline(usize),
    /// Reconciling changes made while no session watched, with paths examined.
    Reconciling(usize),
    Ready,
    Failed(String),
}

/// Only Windows has a notification helper; elsewhere scans keep the archive current.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Watching {
    Off,
    Starting,
    Active,
    Lost(String),
    /// Notifications are not available; scans keep the archive current.
    Unavailable(String),
}

pub(super) struct Health {
    pub readiness: Readiness,
    /// Incomplete scopes protect known paths and tool mutations while
    /// unreadable or unsupported paths remain unresolved.
    pub selective: Option<String>,
    #[cfg(windows)]
    pub policy: super::policy::Policy,
    #[cfg(windows)]
    pub excluded: Vec<String>,
    pub watching: Watching,
    pub dirty: BTreeSet<String>,
    pub interrupted: BTreeSet<u64>,
    pub rescan: Option<String>,
    pub failure: Option<String>,
    /// Origins of Bash commands running in this process.
    pub bash: Vec<String>,
    /// Background processes still running in each conversation runtime.
    pub background: BTreeMap<u64, usize>,
    /// A background process ended; wake archive maintenance.
    pub ended: bool,
    pub event: Option<Instant>,
    pub marker: u64,
    pub busy: bool,
    pub stopped: bool,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub restarts: u32,
    /// Generation of the current notification helper.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub watcher: u64,
    /// Identity of the scope rules the archive follows; a change from another
    /// process rescans so newly included files are captured.
    pub scope: u64,
    /// Stored object bytes after the last collection attempt.
    pub collected: u64,
    /// Newest named point and sorted pin targets before that attempt.
    pub collection_eligibility: Option<(Option<u64>, Vec<u64>)>,
    /// The last collection, or why it failed.
    pub collection: Option<Result<super::gc::Collection, String>>,
}

impl Health {
    fn has_work(&self) -> bool {
        !self.dirty.is_empty()
            || !self.interrupted.is_empty()
            || self.rescan.is_some()
            || matches!(self.watching, Watching::Lost(_))
    }
}

pub(super) struct Signals {
    health: Mutex<Health>,
    pub changed: Condvar,
}

impl Signals {
    pub fn lock(&self) -> MutexGuard<'_, Health> {
        self.health.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn update(&self, change: impl FnOnce(&mut Health)) {
        change(&mut self.lock());
        self.changed.notify_all();
    }
}

struct Work {
    dirty: BTreeSet<String>,
    interrupted: BTreeSet<u64>,
    rescan: Option<String>,
}

pub struct Workspace {
    pub root: PathBuf,
    pub archive: PathBuf,
    store: Mutex<Store>,
    pub(super) leases: Arc<Leases>,
    pub(super) excluded: Vec<String>,
    pub(super) signals: Arc<Signals>,
    #[cfg(windows)]
    pub(super) watcher: Mutex<Option<super::watch::Watcher>>,
    options: Options,
    pub(super) cancellation: Cancellation,
}

static OPEN: Mutex<Vec<Weak<Workspace>>> = Mutex::new(Vec::new());

/// Opens this process's shared handle on the workspace archive.
pub fn open(home: &Path, root: &Path, options: Options) -> Result<Arc<Workspace>, String> {
    let location = identity::locate(home, root, true)?
        .ok_or("Could not create the workspace checkpoint archive")?;
    let mut open = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    open.retain(|workspace| workspace.strong_count() > 0);
    if let Some(found) = open
        .iter()
        .filter_map(Weak::upgrade)
        .find(|workspace| workspace.archive == location.directory)
    {
        super::lifecycle::touch(&found.archive)?;
        return Ok(found);
    }
    let workspace = Arc::new(Workspace::new(home, location, options)?);
    super::lifecycle::touch(&workspace.archive)?;
    open.push(Arc::downgrade(&workspace));
    drop(open);
    workspace.start();
    Ok(workspace)
}

impl Workspace {
    fn new(home: &Path, location: Location, options: Options) -> Result<Self, String> {
        let mut store = Store::open(location.clone())?;
        let scope = store.policy.id();
        let selective = store
            .index
            .current_coverage()
            .and_then(|(_, reason)| reason.strip_prefix("selective: "))
            .map(str::to_owned);
        // Nothing observed the workspace since the last process exited.
        // Existing history remains readable even when a legacy archive is
        // already full or the filesystem will no longer accept writes.
        let reopened_error = scan::coverage(&mut store, false, "reopened").err();
        let owned = [home.to_path_buf(), location.directory.clone()];
        let excluded = scan::physical_exclusions(&location.root, &owned);
        #[cfg(windows)]
        let (scope_policy, scope_excluded) = (store.policy.clone(), excluded.clone());
        let workspace = Self {
            root: location.root.clone(),
            archive: location.directory.clone(),
            leases: Leases::new(super::layout::leases_directory(&location.directory)),
            store: Mutex::new(store),
            excluded,
            signals: Arc::new(Signals {
                health: Mutex::new(Health {
                    readiness: Readiness::Ready,
                    selective: selective.or_else(|| {
                        Some("on-demand recovery; only captured paths are protected".into())
                    }),
                    #[cfg(windows)]
                    policy: scope_policy,
                    #[cfg(windows)]
                    excluded: scope_excluded,
                    watching: Watching::Off,
                    dirty: BTreeSet::new(),
                    interrupted: BTreeSet::new(),
                    rescan: None,
                    failure: reopened_error,
                    bash: Vec::new(),
                    background: BTreeMap::new(),
                    ended: false,
                    event: None,
                    marker: 0,
                    busy: false,
                    stopped: false,
                    restarts: 0,
                    watcher: 0,
                    scope,
                    collected: 0,
                    collection_eligibility: None,
                    collection: None,
                }),
                changed: Condvar::new(),
            }),
            #[cfg(windows)]
            watcher: Mutex::new(None),
            options,
            cancellation: Cancellation::default(),
        };
        if let Err(error) = super::reconcile::incomplete(&workspace) {
            workspace.signals.update(|health| {
                health.failure = Some(error.clone());
                health.readiness = Readiness::Failed(error);
            });
        }
        Ok(workspace)
    }

    fn start(self: &Arc<Self>) {
        if !self.options.background {
            return;
        }
        let weak = Arc::downgrade(self);
        let signals = self.signals.clone();
        let started = std::thread::Builder::new()
            .name("jecode-checkpoints".into())
            .spawn(move || worker(weak, signals));
        if let Err(error) = started {
            self.signals.update(|health| {
                health.readiness = Readiness::Failed(format!("could not start capture: {error}"));
            });
        }
    }

    pub(super) fn store(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn store_cancellable(
        &self,
        cancellation: &Cancellation,
    ) -> Result<MutexGuard<'_, Store>, String> {
        loop {
            if cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            match self.store.try_lock() {
                Ok(store) => return Ok(store),
                Err(std::sync::TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => {
                    cancellation.wait(Duration::from_millis(10))?
                }
            }
        }
    }

    pub fn options(&self) -> Options {
        self.options
    }

    /// Runs the first capture, or reconciles offline changes, on this thread.
    pub fn initialize(&self) -> Result<Summary, String> {
        #[cfg(windows)]
        if self.options.watch {
            self.start_watcher();
        }
        let baseline = !self.store().index.has_baseline();
        self.signals.update(|health| {
            health.readiness = if baseline {
                Readiness::Baseline(0)
            } else {
                Readiness::Reconciling(0)
            };
        });
        let origin = if baseline { "baseline" } else { "external" };
        let result: Result<Summary, String> = (|| {
            let summary = scan::scan_full(self, origin, true, &self.cancellation)?;
            if !self.polling() {
                self.settle();
                self.flush(origin)?;
            }
            let (kind, label) = if baseline {
                ("baseline", "workspace baseline".to_string())
            } else {
                (
                    "reconcile",
                    format!("{} offline changes", summary.changed.len()),
                )
            };
            let record = baseline || !summary.changed.is_empty();
            self.covered_point(
                &summary,
                record.then_some((kind, label.as_str(), origin, false)),
            )?;
            Ok(summary)
        })();
        self.signals.update(|health| {
            health.readiness = match &result {
                Ok(_) => Readiness::Ready,
                Err(error) => Readiness::Failed(error.clone()),
            };
        });
        result
    }

    /// Captures reported changes. One capture runs at a time per process.
    pub(super) fn flush(&self, origin: &str) -> Result<(), String> {
        let work = {
            let mut health = self.signals.lock();
            while health.busy {
                health = self
                    .signals
                    .changed
                    .wait(health)
                    .unwrap_or_else(|e| e.into_inner());
            }
            health.busy = true;
            Work {
                dirty: std::mem::take(&mut health.dirty),
                interrupted: std::mem::take(&mut health.interrupted),
                rescan: health.rescan.take(),
            }
        };
        let interrupted = work.interrupted.clone();
        let result = self.process(work, origin);
        self.signals.update(|health| {
            health.busy = false;
            match &result {
                Ok(()) => health.failure = None,
                Err(error) => {
                    health.interrupted.extend(interrupted);
                    // Work was taken before processing. A failed partial capture
                    // needs a fresh walk even if no further event arrives.
                    health.rescan = Some("a checkpoint capture failed".into());
                    health.failure = Some(error.clone());
                }
            }
        });
        result
    }

    fn process(&self, work: Work, origin: &str) -> Result<(), String> {
        if !work.interrupted.is_empty() {
            self.store().transaction(false, |tx| {
                for operation in &work.interrupted {
                    if tx
                        .index()
                        .pending
                        .get(operation)
                        .is_some_and(|prepare| prepare.owner == tx.store.owner)
                    {
                        let seq = tx.seq();
                        tx.record(super::record::Record::Complete {
                            seq,
                            operation: *operation,
                            outcome: super::record::Outcome::Unknown,
                        });
                    }
                }
                Ok(())
            })?;
        }
        if let Some(reason) = work.rescan {
            scan::coverage(&mut self.store(), false, &reason)?;
            // Unobserved edits may have preserved size and modification time.
            let summary = scan::scan(self, origin, self.options.verify, &self.cancellation)?;
            self.covered_point(&summary, None)?;
            return Ok(());
        }
        if work.dirty.is_empty() {
            return Ok(());
        }
        let policy = self.store().policy.clone();
        let candidates = self.expand(work.dirty, &policy);
        let mut summary = Summary::default();
        scan::capture_paths(
            self,
            &policy,
            candidates,
            scan::CaptureKind::Notification,
            origin,
            &self.cancellation,
            &mut summary,
        )
    }

    /// Reported paths in scope, with the subtrees of new directories and the
    /// recorded descendants of removed ones.
    fn expand(
        &self,
        dirty: BTreeSet<String>,
        policy: &super::policy::Policy,
    ) -> Vec<(String, u64)> {
        let mut paths = BTreeSet::new();
        let selective = self.selective().is_some();
        for path in dirty {
            let metadata = std::fs::symlink_metadata(identity::absolute(&self.root, &path));
            let directory = metadata.as_ref().is_ok_and(|metadata| metadata.is_dir());
            if !scan::in_scope(policy, &self.excluded, &path, directory) {
                continue;
            }
            let prefix = format!("{path}/");
            let store = self.store();
            let recorded = store.index.paths.contains_key(&path);
            let recorded_directory = matches!(
                store.index.current(&path).map(|version| &version.state),
                Some(State::Directory { .. })
            );
            if metadata.is_err() || !directory {
                for (descendant, versions) in store.index.paths.range(prefix.clone()..) {
                    if !descendant.starts_with(&prefix) {
                        break;
                    }
                    if versions
                        .last()
                        .is_some_and(|version| version.state.exists())
                    {
                        paths.insert(descendant.clone());
                    }
                }
            }
            drop(store);
            if selective && !recorded {
                // The parent itself can be unknown while its descendants
                // were protected by write/edit. Keep those observations.
                continue;
            }
            if directory && !recorded_directory && !selective {
                let walk = scan::walk(&self.root, &path, policy, &self.excluded);
                if let Some(reason) = walk.limited {
                    let _ = self.limit_scope(&reason);
                    continue;
                }
                for entry in walk.entries {
                    paths.insert(entry.path);
                }
            }
            paths.insert(path);
        }
        paths.into_iter().map(|path| (path, 0)).collect()
    }

    pub fn set_background(&self, session: u64, running: usize) {
        self.signals.update(|health| {
            let previous = health.background.get(&session).copied().unwrap_or(0);
            health.ended |= running < previous;
            if running == 0 {
                health.background.remove(&session);
            } else {
                health.background.insert(session, running);
            }
        });
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.signals.update(|health| health.stopped = true);
    }
}

fn worker(weak: Weak<Workspace>, signals: Arc<Signals>) {
    loop {
        let work = {
            let mut health = signals.lock();
            if !health.has_work() && !health.stopped {
                health = signals
                    .changed
                    .wait_timeout(health, TICK)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            if health.stopped {
                return;
            }
            health.has_work()
        };
        let Some(workspace) = weak.upgrade() else {
            return;
        };
        if work {
            // Let a burst of notifications arrive so one capture covers it.
            std::thread::sleep(DEBOUNCE);
            #[cfg(windows)]
            workspace.restart_watcher();
            // Notifications identify paths, not the process that changed them.
            if workspace.flush("external").is_err() {
                std::thread::sleep(TICK);
            }
        }
        signals.update(|health| health.ended = false);
        workspace.maintain();
        let _ = super::lifecycle::sweep_throttled(
            workspace.archive.parent().unwrap_or(&workspace.archive),
        );
    }
}
