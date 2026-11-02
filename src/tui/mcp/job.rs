use crate::{
    cancel::Cancellation,
    mcp::{self, AuthMethod, Catalog, Connection, Error, ErrorKind},
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc::Sender},
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

pub(super) type InventoryResult = (String, Option<Connection>, Result<Catalog, ErrorKind>);

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

    pub(super) fn inventory(
        home: PathBuf,
        connections: Vec<Connection>,
        updates: Sender<InventoryResult>,
    ) -> Self {
        Self::start(move |cancel| {
            let queue = Arc::new(Mutex::new(
                connections
                    .into_iter()
                    .filter(|c| c.enabled)
                    .map(|c| c.id)
                    .collect::<VecDeque<_>>(),
            ));
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
                            let Some(id) = queue.lock().unwrap().pop_front() else {
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
                            if updates.send((id, connection, result)).is_err() {
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

    pub(super) fn save(home: PathBuf, connection: Connection) -> Self {
        Self::start(move |cancel| {
            ResultValue::Saved((|| {
                let catalog = mcp::Client::new(connection.clone()).catalog(&cancel)?;
                if cancel.requested() {
                    return Err(Error::new(
                        ErrorKind::Cancelled,
                        "Connection save cancelled",
                    ));
                }
                let saved = mcp::Store::new(home).save(&connection)?;
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
