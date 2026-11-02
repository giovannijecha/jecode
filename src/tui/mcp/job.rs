use crate::{
    cancel::Cancellation,
    mcp::{self, AuthMethod, Catalog, Connection, Error, ErrorKind},
};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
};

pub(super) enum ResultValue {
    List(Result<Vec<Connection>, Error>),
    Inventory,
    Methods(Result<Vec<AuthMethod>, Error>),
    Probe(String, Option<Connection>, Result<Catalog, Error>),
    Saved(Result<(Connection, Catalog), Error>),
    SignedIn(Result<Connection, Error>),
    Removed(String, Result<(), Error>),
    Enabled(String, bool, Result<(), Error>),
}

/// A connection's current record, read under the store lock, and its catalog.
pub(super) type Checked = (String, Option<Connection>, Result<Catalog, ErrorKind>);
type Sent = (String, u64, Option<Connection>, Result<Catalog, ErrorKind>);

/// Background availability checks. They outlive navigation between panels and
/// end only when the MCP panels close. A newer check or an authoritative result
/// for the same connection supersedes an older one.
pub(super) struct Checks {
    pending: HashMap<String, u64>,
    generation: u64,
    sender: Sender<Sent>,
    receiver: Receiver<Sent>,
    jobs: Vec<Job>,
}

impl Default for Checks {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            pending: HashMap::new(),
            generation: 0,
            sender,
            receiver,
            jobs: Vec::new(),
        }
    }
}

impl Checks {
    pub(super) fn start(&mut self, home: PathBuf, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        self.generation += 1;
        for id in &ids {
            self.pending.insert(id.clone(), self.generation);
        }
        let targets = ids.into_iter().map(|id| (id, self.generation)).collect();
        self.jobs
            .push(Job::inventory(home, targets, self.sender.clone()));
    }

    pub(super) fn pending(&self, id: &str) -> bool {
        self.pending.contains_key(id)
    }

    #[cfg(test)]
    pub(super) fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// A newer authoritative result replaces whatever an in-flight check finds.
    pub(super) fn forget(&mut self, id: &str) {
        self.pending.remove(id);
    }

    /// Current results only; checks whose worker ended without one stop pending.
    pub(super) fn take(&mut self) -> Vec<Checked> {
        let idle = self.jobs.iter().all(Job::finished);
        let mut current = Vec::new();
        for (id, generation, connection, result) in self.receiver.try_iter() {
            if self.pending.get(&id) == Some(&generation) {
                self.pending.remove(&id);
                current.push((id, connection, result));
            }
        }
        self.jobs.retain(|job| !job.finished());
        if idle {
            self.pending.clear();
        }
        current
    }
}

pub(super) struct Job {
    thread: Option<JoinHandle<ResultValue>>,
    cancel: Cancellation,
}

impl Job {
    fn start(work: impl FnOnce(Cancellation) -> ResultValue + Send + 'static) -> Self {
        let cancel = Cancellation::default();
        let token = cancel.clone();
        Self {
            thread: Some(thread::spawn(move || work(token))),
            cancel,
        }
    }

    fn inventory(home: PathBuf, targets: Vec<(String, u64)>, updates: Sender<Sent>) -> Self {
        Self::start(move |cancel| {
            let queue = Arc::new(Mutex::new(targets.into_iter().collect::<VecDeque<_>>()));
            let count = queue.lock().unwrap().len().min(4);
            let workers = (0..count)
                .map(|_| {
                    let queue = Arc::clone(&queue);
                    let updates = updates.clone();
                    let cancel = cancel.clone();
                    let home = home.clone();
                    thread::spawn(move || {
                        loop {
                            if cancel.requested() {
                                break;
                            }
                            let Some((id, generation)) = queue.lock().unwrap().pop_front() else {
                                break;
                            };
                            let saved = mcp::Store::new(home.clone()).fresh(&id, &cancel);
                            let (connection, result) = match saved {
                                Ok(connection) => {
                                    let result = mcp::Client::new(connection.clone())
                                        .catalog(&cancel)
                                        .map_err(|e| e.kind);
                                    (Some(connection), result)
                                }
                                Err(error) => (None, Err(error.kind)),
                            };
                            if updates.send((id, generation, connection, result)).is_err() {
                                break;
                            }
                        }
                    })
                })
                .collect::<Vec<_>>();
            for worker in workers {
                let _ = worker.join();
            }
            ResultValue::Inventory
        })
    }

    #[cfg(test)]
    pub(super) fn panicking() -> Self {
        Self::start(|_| panic!("isolated MCP worker failure"))
    }

    pub(super) fn list(home: PathBuf) -> Self {
        Self::start(move |_| ResultValue::List(mcp::Store::new(home).list()))
    }

    pub(super) fn methods(connection: Connection) -> Self {
        Self::start(move |cancel| ResultValue::Methods(mcp::auth::methods(&connection, &cancel)))
    }

    pub(super) fn probe(home: PathBuf, id: String) -> Self {
        Self::start(move |cancel| {
            let saved = mcp::Store::new(home).fresh(&id, &cancel);
            let (connection, result) = match saved {
                Ok(connection) => {
                    let result = mcp::Client::new(connection.clone()).catalog(&cancel);
                    (Some(connection), result)
                }
                Err(error) => (None, Err(error)),
            };
            ResultValue::Probe(id, connection, result)
        })
    }

    /// Check and save a draft. `kept` marks OAuth tokens loaded with the saved
    /// connection rather than obtained by a new sign-in: a refresh may have
    /// rotated them since, so the check uses the current stored tokens and the
    /// save keeps them.
    pub(super) fn save(home: PathBuf, connection: Connection, kept: bool) -> Self {
        Self::start(move |cancel| {
            ResultValue::Saved((|| {
                let store = mcp::Store::new(home);
                let mut check = connection.clone();
                if kept && connection.enabled {
                    check.oauth = store.fresh(&connection.id, &cancel)?.oauth;
                }
                let catalog = mcp::Client::new(check).catalog(&cancel)?;
                if cancel.requested() {
                    return Err(Error::new(
                        ErrorKind::Cancelled,
                        "Connection save cancelled",
                    ));
                }
                let saved = if kept {
                    store.save_edit(&connection)?
                } else {
                    store.save(&connection)?
                };
                Ok((saved, catalog))
            })())
        })
    }

    pub(super) fn sign_in(connection: Connection, updates: Sender<mcp::auth::Update>) -> Self {
        Self::start(move |cancel| {
            ResultValue::SignedIn(mcp::auth::sign_in(connection, &cancel, &mut |update| {
                updates
                    .send(update)
                    .map_err(|_| "Sign-in cancelled".to_string())
            }))
        })
    }

    pub(super) fn remove(home: PathBuf, id: String) -> Self {
        Self::start(move |_| {
            let result = mcp::Store::new(home).remove(&id);
            ResultValue::Removed(id, result)
        })
    }

    pub(super) fn enabled(home: PathBuf, id: String, enabled: bool) -> Self {
        Self::start(move |_| {
            let result = mcp::Store::new(home).set_enabled(&id, enabled);
            ResultValue::Enabled(id, enabled, result)
        })
    }

    pub(super) fn finished(&self) -> bool {
        self.thread.as_ref().is_some_and(JoinHandle::is_finished)
    }
    pub(super) fn finish(mut self) -> Result<ResultValue, String> {
        self.thread
            .take()
            .expect("MCP job thread")
            .join()
            .map_err(|_| "MCP worker failed".into())
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.cancel();
        // Remote calls can outlive cancellation. A closed panel must never block input.
        self.thread.take();
    }
}
